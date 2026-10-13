//! TLS certificate metadata.
//!
//! The handshake uses a verifier that runs normal WebPKI validation (Mozilla roots), records
//! the verdict, and then accepts the certificate anyway. That lets os-int report on expired,
//! self-signed, or mismatched certificates. No application data is sent on the connection.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    self, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};
use x509_parser::public_key::PublicKey;

use super::diagnose;
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    TimelineKind, relation,
};
use crate::sanitize::sanitize_line;

const DEFAULT_TLS_PORT: u16 = 443;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SAN_ENTITIES: usize = 60;

pub struct TlsCollector;

impl Collector for TlsCollector {
    fn id(&self) -> &'static str {
        "tls"
    }

    fn label(&self) -> &'static str {
        "TLS"
    }

    fn description(&self) -> &'static str {
        "Certificate chain, validity diagnostics, SANs, issuer, protocol, and cipher"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Domain | TargetKind::Url)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

/// Parsed certificate fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CertificateInfo {
    pub subject: String,
    pub common_name: Option<String>,
    pub issuer: String,
    pub issuer_organization: Option<String>,
    pub serial: String,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub signature_algorithm: String,
    pub public_key: String,
    pub is_ca: bool,
    pub sha256_fingerprint: String,
}

/// The outcome of a handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsObservation {
    pub host: String,
    pub port: u16,
    pub peer_address: Option<String>,
    pub protocol: Option<String>,
    pub cipher_suite: Option<String>,
    pub alpn: Option<String>,
    /// `Ok(())` when the chain validates for the hostname against Mozilla's roots.
    pub validation: Result<(), String>,
    pub chain: Vec<CertificateInfo>,
}

impl Default for TlsObservation {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 443,
            peer_address: None,
            protocol: None,
            cipher_suite: None,
            alpn: None,
            validation: Ok(()),
            chain: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    verdict: Mutex<Option<Result<(), String>>>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verdict = self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map(|_| ())
            .map_err(|error| error.to_string());
        if let Ok(mut slot) = self.verdict.lock() {
            *slot = Some(verdict);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let (host, port) = match target {
        Target::Domain(domain) => (domain.clone(), DEFAULT_TLS_PORT),
        Target::Url(url) => {
            let host = url
                .host_str()
                .ok_or("the URL has no host")?
                .trim_matches(['[', ']'])
                .to_string();
            let port = if url.scheme() == "https" {
                url.port_or_known_default().unwrap_or(DEFAULT_TLS_PORT)
            } else {
                DEFAULT_TLS_PORT
            };
            (host, port)
        }
        _ => {
            return Err(format!(
                "the TLS collector does not accept {}",
                target.kind()
            ));
        }
    };
    context
        .job
        .progress(format!("TLS handshake with {host}:{port}"));
    context
        .job
        .command("TLS handshake", &[format!("{host}:{port}")]);
    let observation = tokio::select! {
        _ = context.job.cancel.cancelled() => return Err("cancelled".into()),
        result = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&host, port)) => match result {
            Ok(observation) => observation?,
            Err(_) => return Err(format!("TLS collection failed: handshake with {host}:{port} timed out")),
        },
    };
    Ok(CollectionOutput::new(build_batch(target, &observation)))
}

async fn handshake(host: &str, port: u16) -> Result<TlsObservation, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
        .build()
        .map_err(|error| format!("could not create certificate verifier: {error}"))?;
    let verifier = Arc::new(RecordingVerifier {
        inner,
        verdict: Mutex::new(None),
    });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("could not configure TLS: {error}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|error| format!("invalid server name {host:?}: {error}"))?;
    let tcp = TcpStream::connect((host, port)).await.map_err(|error| {
        format!("TLS collection failed: could not connect to {host}:{port}: {error}")
    })?;
    let peer_address = tcp.peer_addr().ok().map(|address| address.to_string());
    let stream = TlsConnector::from(Arc::new(config))
        .connect(server_name, tcp)
        .await
        .map_err(|error| {
            format!("TLS collection failed: handshake with {host}:{port} failed: {error}")
        })?;
    let (_, connection) = stream.get_ref();
    let chain = connection
        .peer_certificates()
        .map(|certificates| {
            certificates
                .iter()
                .map(|der| parse_certificate(der.as_ref()))
                .collect()
        })
        .unwrap_or_default();
    let validation = verifier
        .verdict
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| Err("no certificate was presented".into()));
    Ok(TlsObservation {
        host: host.to_string(),
        port,
        peer_address,
        protocol: connection
            .protocol_version()
            .map(|version| format!("{version:?}")),
        cipher_suite: connection
            .negotiated_cipher_suite()
            .map(|suite| format!("{:?}", suite.suite())),
        alpn: connection
            .alpn_protocol()
            .map(|protocol| String::from_utf8_lossy(protocol).into_owned()),
        validation,
        chain,
    })
}

fn signature_algorithm_name(oid: &str) -> String {
    let name = match oid {
        "1.2.840.113549.1.1.11" => "sha256WithRSAEncryption",
        "1.2.840.113549.1.1.12" => "sha384WithRSAEncryption",
        "1.2.840.113549.1.1.13" => "sha512WithRSAEncryption",
        "1.2.840.113549.1.1.5" => "sha1WithRSAEncryption",
        "1.2.840.10045.4.3.2" => "ecdsa-with-SHA256",
        "1.2.840.10045.4.3.3" => "ecdsa-with-SHA384",
        "1.3.101.112" => "Ed25519",
        _ => return oid.to_string(),
    };
    name.to_string()
}

/// Parses a DER certificate. Unparseable certificates still yield a fingerprint.
pub fn parse_certificate(der: &[u8]) -> CertificateInfo {
    let fingerprint = sha256_hex(der);
    let Ok((_, certificate)) = X509Certificate::from_der(der) else {
        return CertificateInfo {
            subject: "(unparseable certificate)".into(),
            sha256_fingerprint: fingerprint,
            ..CertificateInfo::default()
        };
    };
    let mut info = CertificateInfo {
        subject: sanitize_line(&certificate.subject().to_string()).into_owned(),
        common_name: certificate
            .subject()
            .iter_common_name()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(|name| sanitize_line(name).into_owned()),
        issuer: sanitize_line(&certificate.issuer().to_string()).into_owned(),
        issuer_organization: certificate
            .issuer()
            .iter_organization()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(|name| sanitize_line(name).into_owned()),
        serial: certificate.raw_serial_as_string(),
        not_before: DateTime::from_timestamp(certificate.validity().not_before.timestamp(), 0),
        not_after: DateTime::from_timestamp(certificate.validity().not_after.timestamp(), 0),
        signature_algorithm: signature_algorithm_name(
            &certificate.signature_algorithm.algorithm.to_id_string(),
        ),
        public_key: describe_public_key(&certificate),
        is_ca: certificate.is_ca(),
        sha256_fingerprint: fingerprint,
        ..CertificateInfo::default()
    };
    if let Ok(Some(names)) = certificate.subject_alternative_name() {
        for name in &names.value.general_names {
            match name {
                GeneralName::DNSName(dns) => info
                    .dns_names
                    .push(sanitize_line(&dns.to_ascii_lowercase()).into_owned()),
                GeneralName::IPAddress(bytes) => {
                    if let Some(address) = ip_from_bytes(bytes) {
                        info.ip_addresses.push(address.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    info
}

fn describe_public_key(certificate: &X509Certificate<'_>) -> String {
    match certificate.public_key().parsed() {
        Ok(PublicKey::RSA(key)) => format!("RSA {} bits", key.key_size()),
        Ok(PublicKey::EC(point)) => format!("EC {} bits", point.key_size()),
        Ok(_) => "other".into(),
        Err(_) => "unparsed".into(),
    }
}

fn ip_from_bytes(bytes: &[u8]) -> Option<IpAddr> {
    match bytes.len() {
        4 => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
        16 => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
        _ => None,
    }
}

fn format_date(date: &Option<DateTime<Utc>>) -> String {
    date.map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn describe(
    observation: &TlsObservation,
    diagnostics: &[diagnose::Diagnostic],
) -> (String, Fields) {
    let mut lines = vec![format!(
        "Endpoint: {}:{}",
        observation.host, observation.port
    )];
    let mut fields = Fields::new();
    if let Some(address) = &observation.peer_address {
        lines.push(format!("Peer address: {address}"));
    }
    lines.push(format!(
        "Protocol: {}",
        observation.protocol.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "Cipher suite: {}",
        observation.cipher_suite.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "ALPN: {}",
        observation.alpn.as_deref().unwrap_or("none")
    ));
    let validation = match &observation.validation {
        Ok(()) => "valid for this hostname (Mozilla root store)".to_string(),
        Err(error) => format!("NOT trusted: {error}"),
    };
    lines.push(format!("Validation: {validation}"));
    fields.push("validation", validation);
    diagnose::append_section(&mut lines, diagnostics);
    let posture = diagnose::posture(diagnostics);
    if !posture.is_empty() {
        fields.push("diagnostics", posture);
    }
    fields.push_opt("protocol", observation.protocol.clone());
    for (position, certificate) in observation.chain.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!(
            "Certificate #{position}{}",
            if position == 0 { " (leaf)" } else { "" }
        ));
        lines.push(format!("  Subject: {}", certificate.subject));
        lines.push(format!("  Issuer: {}", certificate.issuer));
        lines.push(format!("  Serial: {}", certificate.serial));
        lines.push(format!(
            "  Valid: {} → {}",
            format_date(&certificate.not_before),
            format_date(&certificate.not_after)
        ));
        lines.push(format!(
            "  Key: {}  Signature: {}",
            certificate.public_key, certificate.signature_algorithm
        ));
        lines.push(format!("  SHA-256: {}", certificate.sha256_fingerprint));
        if !certificate.dns_names.is_empty() {
            lines.push(format!(
                "  DNS names ({}): {}",
                certificate.dns_names.len(),
                certificate.dns_names.join(", ")
            ));
        }
        if !certificate.ip_addresses.is_empty() {
            lines.push(format!(
                "  IP addresses: {}",
                certificate.ip_addresses.join(", ")
            ));
        }
    }
    if let Some(leaf) = observation.chain.first() {
        fields.push("subject", leaf.subject.clone());
        fields.push("issuer", leaf.issuer.clone());
        fields.push("not_before", format_date(&leaf.not_before));
        fields.push("not_after", format_date(&leaf.not_after));
        fields.push("sha256", leaf.sha256_fingerprint.clone());
        fields.push("san_count", leaf.dns_names.len().to_string());
    }
    (lines.join("\n"), fields)
}

/// Builds the evidence batch for a TLS observation. Pure and unit-tested.
pub fn build_batch(target: &Target, observation: &TlsObservation) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let diagnostics = diagnose::assess_tls(
        observation.validation.is_ok(),
        observation.validation.as_ref().err().map(String::as_str),
        observation.chain.first().and_then(|leaf| leaf.not_after),
        Utc::now(),
    );
    let (content, fields) = describe(observation, &diagnostics);
    let mut source = NewSource::collected(
        SourceKind::Tls,
        "tls",
        format!(
            "TLS certificate for {}:{}",
            observation.host, observation.port
        ),
    );
    let fingerprints: Vec<&str> = observation
        .chain
        .iter()
        .map(|certificate| certificate.sha256_fingerprint.as_str())
        .collect();
    source.content_hash = Some(sha256_hex(fingerprints.join(",").as_bytes()));
    source.content = content;
    source.fields = fields;
    let key = batch.add_source(source);
    diagnose::record_findings(
        &mut batch,
        &format!("{}:{}", observation.host, observation.port),
        &diagnostics,
        Some(key),
    );
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    let Some(leaf) = observation.chain.first() else {
        batch.summary = "no certificate presented".into();
        return batch;
    };
    let certificate = batch.entity(EntityKind::Certificate, &leaf.sha256_fingerprint, Some(key));
    batch.label(
        &certificate,
        leaf.common_name
            .clone()
            .unwrap_or_else(|| leaf.subject.clone()),
    );
    batch.annotate(&certificate, "issuer", leaf.issuer.clone());
    batch.annotate(&certificate, "not_after", format_date(&leaf.not_after));
    batch.relate(
        &subject,
        relation::PRESENTS_CERTIFICATE,
        &certificate,
        Some(key),
    );
    if let Some(organization) = &leaf.issuer_organization {
        let issuer = EntityKey::new(EntityKind::Organization, organization);
        batch.relate(&certificate, relation::ISSUED_BY, &issuer, Some(key));
    }
    let own_host = target.hostname().unwrap_or_default();
    for name in leaf.dns_names.iter().take(MAX_SAN_ENTITIES) {
        let is_wildcard = name.starts_with("*.");
        let value = name.trim_start_matches("*.");
        if value == own_host {
            continue;
        }
        let kind = if is_wildcard {
            EntityKind::Domain
        } else {
            EntityKind::Hostname
        };
        let covered = EntityKey::new(kind, value);
        batch.relate(&certificate, relation::COVERS, &covered, Some(key));
    }
    if let Some(not_before) = leaf.not_before {
        batch.event(
            not_before,
            TimelineKind::CertificateIssued,
            format!("Certificate for {} valid from", observation.host),
            Some(key),
            Some(certificate.clone()),
        );
    }
    if let Some(not_after) = leaf.not_after {
        batch.event(
            not_after,
            TimelineKind::CertificateExpires,
            format!("Certificate for {} expires", observation.host),
            Some(key),
            Some(certificate),
        );
    }
    batch.summary = format!(
        "{} certificate from {} valid until {}{}",
        observation.protocol.as_deref().unwrap_or("TLS"),
        leaf.issuer_organization.as_deref().unwrap_or(&leaf.issuer),
        format_date(&leaf.not_after),
        if observation.validation.is_ok() {
            ""
        } else {
            " (NOT trusted)"
        }
    );
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_observation() -> TlsObservation {
        TlsObservation {
            host: "example.com".into(),
            port: 443,
            peer_address: Some("192.0.2.5:443".into()),
            protocol: Some("TLSv1_3".into()),
            cipher_suite: Some("TLS13_AES_128_GCM_SHA256".into()),
            alpn: Some("h2".into()),
            validation: Ok(()),
            chain: vec![CertificateInfo {
                subject: "CN=example.com".into(),
                common_name: Some("example.com".into()),
                issuer: "C=US, O=Example CA, CN=Example Issuing CA".into(),
                issuer_organization: Some("Example CA".into()),
                serial: "01:02".into(),
                not_before: DateTime::from_timestamp(1_700_000_000, 0),
                not_after: DateTime::from_timestamp(1_730_000_000, 0),
                dns_names: vec![
                    "example.com".into(),
                    "www.example.com".into(),
                    "*.example.com".into(),
                    "api.example.org".into(),
                ],
                ip_addresses: Vec::new(),
                signature_algorithm: "ecdsa-with-SHA256".into(),
                public_key: "EC 256 bits".into(),
                is_ca: false,
                sha256_fingerprint: "ab".repeat(32),
            }],
        }
    }

    #[test]
    fn batch_links_certificate_issuer_and_sans() {
        let target = Target::Domain("example.com".into());
        let actual = build_batch(&target, &sample_observation());
        let covered: Vec<&str> = actual
            .relationships
            .iter()
            .filter(|relationship| relationship.kind == "covers")
            .map(|relationship| relationship.to.value.as_str())
            .collect();
        assert_eq!(covered, vec!["www.example.com", "api.example.org"]);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "issued_by")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "presents_certificate")
        );
        assert_eq!(actual.timeline.len(), 2);
        assert!(actual.summary.contains("Example CA"));
    }

    #[test]
    fn untrusted_certificates_are_flagged() {
        let mut input = sample_observation();
        input.validation = Err("certificate expired".into());
        let actual = build_batch(&Target::Domain("example.com".into()), &input);
        assert!(actual.summary.contains("NOT trusted"));
        assert!(
            actual.sources[0]
                .content
                .contains("NOT trusted: certificate expired")
        );
    }

    #[test]
    fn garbage_der_still_yields_fingerprint() {
        let actual = parse_certificate(b"not a certificate");
        assert_eq!(actual.subject, "(unparseable certificate)");
        assert_eq!(actual.sha256_fingerprint.len(), 64);
    }

    #[test]
    fn signature_oids_are_named() {
        assert_eq!(
            signature_algorithm_name("1.2.840.10045.4.3.2"),
            "ecdsa-with-SHA256"
        );
        assert_eq!(signature_algorithm_name("1.2.3"), "1.2.3");
    }
}
