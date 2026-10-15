//! Optional user-configured API connectors (URLScan search, OTX pulse lookup).
//!
//! Keys come from `config.toml` or an environment variable. They are never written into
//! source content or job logs. Connectors stay off until the analyst enables them.

use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::Value;
use url::Url;

use super::net::{FetchOptions, fetch};
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::config::{ApiConnectorConfig, duration_from_secs};
use crate::exec::redact::redact_value;
use crate::files::sha256_hex;
use crate::model::{
    EntityKind, EvidenceBatch, NewSource, SourceKind, Target, TargetKind, relation,
};
use crate::sanitize::sanitize_text;

const DEFAULT_RATE_LIMIT: u32 = 30;

/// A thin HTTP wrapper around a user-configured intelligence API.
#[derive(Clone, Debug)]
pub struct ApiConnector {
    id: String,
    label: String,
    description: String,
    accepts: Vec<TargetKind>,
    api_key: Option<String>,
    rate_limit_per_minute: u32,
    timeout: Duration,
}

impl ApiConnector {
    pub fn from_config(config: &ApiConnectorConfig) -> Result<Self, String> {
        if !config.enabled {
            return Err("disabled".into());
        }
        let id = config.id.trim().to_ascii_lowercase();
        let (label, description, accepts) = match id.as_str() {
            "urlscan" => (
                "URLScan",
                "URLScan search (user-configured)",
                vec![TargetKind::Domain, TargetKind::Url, TargetKind::Ip],
            ),
            "otx" => (
                "OTX",
                "OTX indicator lookup (user-configured API key)",
                vec![TargetKind::Domain, TargetKind::Ip, TargetKind::Url],
            ),
            other => {
                return Err(format!(
                    "unknown API connector {other:?}; built-in ids are urlscan and otx"
                ));
            }
        };
        let api_key = resolve_api_key(config);
        if id == "otx" && api_key.is_none() {
            return Err("OTX connector needs api_key or api_key_env".into());
        }
        Ok(Self {
            id,
            label: label.into(),
            description: description.into(),
            accepts,
            api_key,
            rate_limit_per_minute: if config.rate_limit_per_minute == 0 {
                DEFAULT_RATE_LIMIT
            } else {
                config.rate_limit_per_minute
            },
            timeout: duration_from_secs(config.timeout_seconds.max(1)),
        })
    }
}

fn resolve_api_key(config: &ApiConnectorConfig) -> Option<String> {
    let inline = config.api_key.trim();
    if !inline.is_empty() {
        return Some(inline.to_string());
    }
    let env_name = config.api_key_env.trim();
    if env_name.is_empty() {
        return None;
    }
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl Collector for ApiConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        self.accepts.contains(&kind)
    }

    fn runs_by_default(&self) -> bool {
        false
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(self, target, context))
    }
}

async fn collect(
    connector: &ApiConnector,
    target: &Target,
    context: &CollectorContext,
) -> CollectResult {
    let delay = 60_000 / connector.rate_limit_per_minute.max(1);
    tokio::time::sleep(Duration::from_millis(u64::from(delay.min(1_000)))).await;
    let work = async {
        match connector.id.as_str() {
            "urlscan" => collect_urlscan(connector, target, context).await,
            "otx" => collect_otx(connector, target, context).await,
            other => Err(format!("unknown API connector {other}")),
        }
    };
    match tokio::time::timeout(connector.timeout, work).await {
        Ok(result) => result,
        Err(_) => Err(format!("{} timed out", connector.id)),
    }
}

async fn collect_urlscan(
    connector: &ApiConnector,
    target: &Target,
    context: &CollectorContext,
) -> CollectResult {
    let query = match target {
        Target::Domain(domain) => format!("domain:{domain}"),
        Target::Ip(ip) => format!("ip:{ip}"),
        Target::Url(url) => format!("page.url:\"{url}\""),
        other => other.value(),
    };
    let url = Url::parse_with_params(
        "https://urlscan.io/api/v1/search/",
        &[("q", query.as_str()), ("size", "20")],
    )
    .map_err(|error| format!("invalid URLScan query: {error}"))?;
    context.job.progress("Querying URLScan");
    context.job.command("GET", &[redact_value(url.as_str())]);
    let response = fetch_with_key(context, &url, connector.api_key.as_deref(), "API-Key").await?;
    let json: Value = serde_json::from_slice(&response)
        .map_err(|error| format!("URLScan returned invalid JSON ({error})"))?;
    Ok(CollectionOutput::new(build_urlscan_batch(
        target,
        &json,
        connector.api_key.as_deref(),
    )))
}

async fn collect_otx(
    connector: &ApiConnector,
    target: &Target,
    context: &CollectorContext,
) -> CollectResult {
    let (kind, value) = match target {
        Target::Domain(domain) => ("domain", domain.clone()),
        Target::Ip(ip) => ("IPv4", ip.to_string()),
        Target::Url(url) => ("url", url.to_string()),
        other => ("hostname", other.value()),
    };
    let url = Url::parse(&format!(
        "https://otx.alienvault.com/api/v1/indicators/{kind}/{value}/general"
    ))
    .map_err(|error| format!("invalid OTX URL: {error}"))?;
    context.job.progress("Querying OTX");
    context.job.command("GET", &[redact_value(url.as_str())]);
    let body = fetch_with_key(context, &url, connector.api_key.as_deref(), "X-OTX-API-KEY").await?;
    let json: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("OTX returned invalid JSON ({error}"))?;
    Ok(CollectionOutput::new(build_otx_batch(
        target,
        &json,
        connector.api_key.as_deref(),
    )))
}

async fn fetch_with_key(
    context: &CollectorContext,
    url: &Url,
    api_key: Option<&str>,
    header_name: &str,
) -> Result<Vec<u8>, String> {
    let mut extra_headers = Vec::new();
    if let Some(key) = api_key {
        extra_headers.push((header_name.to_string(), key.to_string()));
    }
    let options = FetchOptions {
        max_body_bytes: context.net.settings.max_body_bytes,
        accept: Some("application/json"),
        extra_headers,
    };
    let response = fetch(context.net.as_ref(), url, &options, &context.job.cancel).await?;
    if let Some(reason) = response.blocked_redirect {
        return Err(reason);
    }
    if response.status == 401 || response.status == 403 {
        return Err(format!(
            "API connector refused the configured key (HTTP {})",
            response.status
        ));
    }
    if !response.is_success() {
        return Err(format!("API connector failed: HTTP {}", response.status));
    }
    Ok(response.body)
}

/// Pure mapping of a URLScan search payload.
pub fn build_urlscan_batch(target: &Target, json: &Value, api_key: Option<&str>) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(
        SourceKind::Web,
        "api:urlscan",
        format!("URLScan search for {}", target.value()),
    );
    source.url = Some("https://urlscan.io/api/v1/search/".into());
    let rendered = redact_secrets(&json.to_string(), api_key);
    source.content = sanitize_text(&rendered).into_owned();
    source.content_hash = Some(sha256_hex(source.content.as_bytes()));
    source.fields.push("connector", "urlscan");
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    if let Some(results) = json.get("results").and_then(Value::as_array) {
        for result in results.iter().take(20) {
            if let Some(url) = result.pointer("/page/url").and_then(Value::as_str) {
                let url_key = batch.entity(EntityKind::Url, url, Some(key));
                batch.relate(&subject, relation::REFERENCES, &url_key, Some(key));
            }
            if let Some(domain) = result.pointer("/page/domain").and_then(Value::as_str) {
                let domain_key = batch.entity(EntityKind::Domain, domain, Some(key));
                if domain_key != subject {
                    batch.relate(&subject, relation::ASSOCIATED_WITH, &domain_key, Some(key));
                }
            }
            if let Some(ip) = result.pointer("/page/ip").and_then(Value::as_str) {
                let ip_key = batch.entity(EntityKind::Ip, ip, Some(key));
                batch.relate(&subject, relation::RESOLVES_TO, &ip_key, Some(key));
            }
        }
    }
    batch.summary = format!(
        "URLScan: {} related object(s)",
        batch.entities.len().saturating_sub(1)
    );
    batch
}

/// Pure mapping of an OTX general indicator payload.
pub fn build_otx_batch(target: &Target, json: &Value, api_key: Option<&str>) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(
        SourceKind::Web,
        "api:otx",
        format!("OTX indicator for {}", target.value()),
    );
    source.url = Some("https://otx.alienvault.com/".into());
    source.content = sanitize_text(&redact_secrets(&json.to_string(), api_key)).into_owned();
    source.content_hash = Some(sha256_hex(source.content.as_bytes()));
    source.fields.push("connector", "otx");
    if let Some(pulse) = json.get("pulse_info").and_then(|info| info.get("count")) {
        source.fields.push("pulse_count", pulse.to_string());
    }
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    if let Some(sections) = json.get("sections").and_then(Value::as_array) {
        for section in sections {
            if let Some(name) = section.as_str() {
                batch.annotate(&subject, "otx_section", name);
            }
        }
    }
    if let Some(pulses) = json.pointer("/pulse_info/pulses").and_then(Value::as_array) {
        for pulse in pulses.iter().take(15) {
            if let Some(name) = pulse.get("name").and_then(Value::as_str) {
                let org = batch.entity(EntityKind::Unknown, name, Some(key));
                batch.relate(&subject, relation::ASSOCIATED_WITH, &org, Some(key));
            }
        }
    }
    batch.summary = format!("OTX indicator for {}", target.value());
    batch
}

fn redact_secrets(text: &str, api_key: Option<&str>) -> String {
    let mut output = text.to_string();
    if let Some(key) = api_key.filter(|key| key.len() >= 4) {
        output = output.replace(key, "[REDACTED]");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::Collector;

    #[test]
    fn urlscan_results_become_references() {
        let json = serde_json::json!({
            "results": [{
                "page": {
                    "url": "https://example.com/a",
                    "domain": "example.com",
                    "ip": "192.0.2.10"
                }
            }]
        });
        let target = Target::Domain("example.com".into());
        let actual = build_urlscan_batch(&target, &json, Some("sk-secret"));
        assert!(actual.sources[0].content.contains("https://example.com/a"));
        assert!(!actual.sources[0].content.contains("sk-secret"));
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "references"
                    || relationship.kind == "resolves_to")
        );
    }

    #[test]
    fn otx_pulses_are_associated() {
        let json = serde_json::json!({
            "pulse_info": {"count": 1, "pulses": [{"name": "toy-pulse"}]}
        });
        let target = Target::Domain("example.com".into());
        let actual = build_otx_batch(&target, &json, None);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.to.value == "toy-pulse")
        );
    }

    #[test]
    fn disabled_or_unknown_connectors_fail_config() {
        let disabled = ApiConnectorConfig {
            id: "urlscan".into(),
            enabled: false,
            ..ApiConnectorConfig::default()
        };
        assert!(ApiConnector::from_config(&disabled).is_err());
        let unknown = ApiConnectorConfig {
            id: "nope".into(),
            enabled: true,
            ..ApiConnectorConfig::default()
        };
        assert!(
            ApiConnector::from_config(&unknown)
                .expect_err("id")
                .contains("unknown")
        );
    }

    #[test]
    fn enabled_connectors_are_not_run_by_default() {
        let config = ApiConnectorConfig {
            id: "urlscan".into(),
            enabled: true,
            ..ApiConnectorConfig::default()
        };
        let connector = ApiConnector::from_config(&config).expect("enabled");
        assert!(!connector.runs_by_default());
        let tools = crate::tools::ToolRegistry::detect(&crate::config::ToolsConfig::default());
        let mut config = crate::config::Config::default();
        config.collectors.api.push(ApiConnectorConfig {
            id: "urlscan".into(),
            enabled: true,
            ..ApiConnectorConfig::default()
        });
        let registry = crate::collectors::CollectorRegistry::from_config(&config);
        let default_collectors = registry.defaults_for(crate::model::TargetKind::Domain, &tools);
        let defaults: Vec<&str> = default_collectors
            .iter()
            .map(|collector| collector.id())
            .collect();
        assert!(
            !defaults.contains(&"urlscan"),
            "enabled API connectors must stay out of the default set: {defaults:?}"
        );
        assert!(registry.find("urlscan").is_some());
    }
}
