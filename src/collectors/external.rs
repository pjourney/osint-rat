//! External collector protocol: a sidecar program that prints Evidence Batch JSON on stdout.
//!
//! Configured as `[[collectors.external]]`. The program is spawned through `exec/` with an
//! explicit argv (no shell). Tools marked active are refused. Stdout is parsed by the ingest
//! plane and applied through the same `storage::apply` path as built-in collectors.

use std::path::PathBuf;
use std::time::Duration;

use futures::future::BoxFuture;

use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::config::{ExternalCollectorConfig, duration_from_secs};
use crate::exec::{CommandSpec, ExecError, run};
use crate::ingest::{BATCH_FORMAT_VERSION, IngestFormat, IngestOptions, ingest};
use crate::model::{Target, TargetKind};
use crate::sanitize::sanitize_line;
use crate::tools::Activity;
use crate::tools::template::{render_args, validate_template};

const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;

/// A user-configured sidecar that emits Ontology Exchange / Evidence Batch JSON.
#[derive(Clone, Debug)]
pub struct ExternalCollector {
    id: String,
    label: String,
    description: String,
    program: String,
    args: Vec<String>,
    accepts: Vec<TargetKind>,
    timeout: Duration,
    activity: Activity,
    runs_by_default: bool,
    working_directory: Option<PathBuf>,
}

impl ExternalCollector {
    pub fn from_config(config: &ExternalCollectorConfig) -> Result<Self, String> {
        let id = config.id.trim();
        if id.is_empty() || config.program.trim().is_empty() {
            return Err("[[collectors.external]] is missing `id` or `program`".into());
        }
        if Activity::parse(&config.activity) == Activity::Active {
            return Err(format!(
                "external collector {id} is marked active; os-int only runs passive observation"
            ));
        }
        let dummy: Vec<String> = config
            .args
            .iter()
            .map(|arg| {
                arg.replace("{script}", "script")
                    .replace("{id}", "id")
                    .replace("{kind}", "kind")
                    .replace("{value}", "value")
            })
            .collect();
        if let Err(error) = validate_template(&dummy) {
            return Err(format!("external collector {id}: {error}"));
        }
        let accepts: Vec<TargetKind> = config
            .accepts
            .iter()
            .filter_map(|kind| TargetKind::parse(kind))
            .collect();
        if accepts.is_empty() {
            return Err(format!(
                "external collector {id}: `accepts` lists no known target kinds"
            ));
        }
        Ok(Self {
            id: id.to_string(),
            label: if config.label.trim().is_empty() {
                id.to_string()
            } else {
                config.label.trim().to_string()
            },
            description: if config.description.trim().is_empty() {
                format!("External connector {id}")
            } else {
                config.description.trim().to_string()
            },
            program: config.program.trim().to_string(),
            args: config.args.clone(),
            accepts,
            timeout: duration_from_secs(config.timeout_seconds.max(1)),
            activity: Activity::Passive,
            runs_by_default: config.runs_by_default,
            working_directory: config
                .working_directory
                .as_ref()
                .filter(|path| !path.as_os_str().is_empty())
                .cloned(),
        })
    }
}

impl Collector for ExternalCollector {
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

    fn activity(&self) -> Activity {
        self.activity
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn runs_by_default(&self) -> bool {
        self.runs_by_default
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
    collector: &ExternalCollector,
    target: &Target,
    context: &CollectorContext,
) -> CollectResult {
    if collector.activity == Activity::Active {
        return Err(format!(
            "{} is marked active. os-int only runs passive observation",
            collector.id
        ));
    }
    let args = render_sidecar_args(&collector.args, target, &collector.id)?;
    let mut spec = CommandSpec::new(collector.program.clone(), args, collector.timeout);
    spec.max_output_bytes = MAX_BATCH_BYTES;
    spec.cwd = collector.working_directory.clone();
    context.job.command(&spec.program, &spec.args);
    context
        .job
        .progress(format!("Running external connector {}", collector.id));
    let outcome = match run(&spec, &context.job.cancel, |line| {
        context.job.log(sanitize_line(&line.text).into_owned());
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(ExecError::Cancelled) => return Err("cancelled".into()),
        Err(error) => return Err(error.to_string()),
    };
    if !outcome.success && outcome.stdout.trim().is_empty() {
        return Err(format!(
            "{} exited {}: {}",
            collector.id,
            outcome.exit_code.unwrap_or(-1),
            sanitize_line(outcome.stderr.lines().next().unwrap_or("no output"))
        ));
    }
    let options = IngestOptions {
        origin: format!("connector:{}", collector.id),
        mapping: crate::ingest::ObservableMapping::default(),
    };
    let report = ingest(IngestFormat::Batch, outcome.stdout.as_bytes(), &options)
        .map_err(|error| error.message)?;
    let mut output = CollectionOutput::new(report.batch);
    output.warnings.extend(report.warnings);
    if output.batch.summary.is_empty() {
        output.batch.summary = format!(
            "{} returned evidence batch format_version {BATCH_FORMAT_VERSION}",
            collector.id
        );
    }
    Ok(output)
}

fn render_sidecar_args(args: &[String], target: &Target, id: &str) -> Result<Vec<String>, String> {
    let rewritten: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.replace("{script}", "")
                .replace("{id}", id)
                .replace("{kind}", target.kind().as_str())
                .replace("{value}", &target.value())
        })
        .collect();
    render_args(&rewritten, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExternalCollectorConfig;

    #[test]
    fn active_external_collectors_are_refused() {
        let input = ExternalCollectorConfig {
            id: "probe".into(),
            program: "probe".into(),
            args: vec!["{target}".into()],
            accepts: vec!["domain".into()],
            activity: "active".into(),
            ..ExternalCollectorConfig::default()
        };
        let actual = ExternalCollector::from_config(&input);
        assert!(actual.expect_err("active").contains("active"));
    }

    #[test]
    fn sidecar_args_substitute_kind_and_target() {
        let target = Target::Domain("example.com".into());
        let actual = render_sidecar_args(
            &[
                "--target".into(),
                "{target}".into(),
                "--kind".into(),
                "{kind}".into(),
            ],
            &target,
            "hello-batch",
        )
        .expect("args");
        assert_eq!(actual, vec!["--target", "example.com", "--kind", "domain"]);
    }

    #[test]
    fn hello_batch_example_applies_without_forking_rust() {
        let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/connectors/hello-batch/hello_batch.py");
        let output = std::process::Command::new("python3")
            .args([
                script.as_os_str(),
                std::ffi::OsStr::new("--target"),
                std::ffi::OsStr::new("example.com"),
                std::ffi::OsStr::new("--kind"),
                std::ffi::OsStr::new("domain"),
            ])
            .output()
            .expect("python3 hello_batch.py");
        assert!(
            output.status.success(),
            "hello-batch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = crate::ingest::parse_batch_json(&output.stdout, "hello-batch").expect("batch");
        assert!(
            report
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "subdomain_of")
        );
        assert!(
            report
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "resolves_to")
        );
        assert_eq!(
            report.batch.sources[0].provenance,
            crate::model::Provenance::External
        );
        let mut store = crate::storage::Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("hello-batch", "").expect("case");
        store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id: investigation.id,
                    page_id: None,
                    job_id: None,
                },
                &report.batch,
            )
            .expect("apply");
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(
            data.entities
                .iter()
                .any(|entity| entity.value == "example.com")
        );
        assert!(
            data.entities
                .iter()
                .any(|entity| entity.value == "www.example.com")
        );
        assert!(
            data.relationships
                .iter()
                .any(|relationship| relationship.kind == "subdomain_of")
        );
        assert!(!data.sources.is_empty());
        assert!(
            data.relationships
                .iter()
                .all(|relationship| !relationship.source_ids.is_empty())
        );
    }
}
