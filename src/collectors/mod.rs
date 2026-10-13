//! Passive collectors.
//!
//! A collector takes a `Target` and returns a `CollectionOutput` (an `EvidenceBatch` of sources,
//! entities, relationships, timeline events, and artifacts). Collectors never write to the
//! database: the caller applies the batch in one transaction, which keeps collectors simple to
//! test and lets the CLI and the TUI share them.
//!
//! To add a collector: implement `Collector`, then register it in `CollectorRegistry::builtin`.

pub mod ct;
pub mod diagnose;
pub mod dns;
pub mod file;
pub mod html;
pub mod http;
pub mod ip;
pub mod net;
pub mod rdap;
pub mod repository;
pub mod tls;
pub mod tool;

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;

pub use net::{NetContext, NetSettings};

use crate::files::InvestigationFiles;
use crate::jobs::JobContext;
use crate::model::{EvidenceBatch, Target, TargetKind};
use crate::tools::{Activity, ToolRegistry};

/// What a collector produced.
#[derive(Debug, Default)]
pub struct CollectionOutput {
    pub batch: EvidenceBatch,
    /// Non-fatal problems (one DNS query timed out, robots.txt missing, ...).
    pub warnings: Vec<String>,
}

impl CollectionOutput {
    pub fn new(batch: EvidenceBatch) -> Self {
        Self {
            batch,
            warnings: Vec::new(),
        }
    }
}

/// Collector settings derived from configuration.
#[derive(Clone, Debug)]
pub struct CollectorSettings {
    pub fetch_robots: bool,
    pub check_www: bool,
    pub repository_history_depth: u32,
    pub max_file_import_bytes: u64,
}

impl Default for CollectorSettings {
    fn default() -> Self {
        let defaults = crate::config::Config::default();
        Self {
            fetch_robots: defaults.collectors.fetch_robots,
            check_www: defaults.collectors.check_www,
            repository_history_depth: defaults.collectors.repository_history_depth,
            max_file_import_bytes: defaults.storage.max_file_import_bytes,
        }
    }
}

/// Everything a collector may use. Cheap to clone.
#[derive(Clone)]
pub struct CollectorContext {
    pub net: Arc<NetContext>,
    pub files: InvestigationFiles,
    pub tools: Arc<ToolRegistry>,
    pub cache_dir: PathBuf,
    pub settings: CollectorSettings,
    pub job: JobContext,
}

/// Result type for collectors; errors are user-facing messages shown as job failures.
pub type CollectResult = Result<CollectionOutput, String>;

pub trait Collector: Send + Sync {
    /// Stable identifier used in configuration, the CLI, and agent actions (`dns`).
    fn id(&self) -> &'static str;
    /// Short label for status displays (`DNS`).
    fn label(&self) -> &'static str;
    /// One line shown in the command palette and job description.
    fn description(&self) -> &'static str;
    /// Whether this collector can run against `kind`.
    ///
    /// The default set for a page is every accepting collector that also returns true from
    /// [`Collector::runs_by_default`] and whose [`Collector::requires`] programs are installed.
    fn accepts(&self, kind: TargetKind) -> bool;
    /// `Passive` for every built-in collector. Active work is refused at the tool boundary.
    fn activity(&self) -> Activity {
        Activity::Passive
    }
    /// External programs required; the collector is unavailable when any is missing.
    fn requires(&self) -> &'static [&'static str] {
        &[]
    }
    /// Whether "run all collectors" includes this collector by default.
    fn runs_by_default(&self) -> bool {
        true
    }
    /// Gather evidence for `target`.
    ///
    /// Return a [`CollectionOutput`] and do not touch SQLite. Put non-fatal problems in
    /// `warnings`. Return `Err` only when the collector produced nothing worth applying.
    /// Check `context.job.cancel` on long loops so the Jobs screen can cancel the task.
    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult>;
}

#[derive(Clone)]
pub struct CollectorRegistry {
    collectors: Vec<Arc<dyn Collector>>,
}

impl std::fmt::Debug for CollectorRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self
            .collectors
            .iter()
            .map(|collector| collector.id())
            .collect();
        formatter
            .debug_struct("CollectorRegistry")
            .field("collectors", &ids)
            .finish()
    }
}

impl CollectorRegistry {
    pub fn builtin() -> Self {
        Self {
            collectors: vec![
                Arc::new(dns::DnsCollector),
                Arc::new(rdap::RdapCollector),
                Arc::new(http::HttpCollector),
                Arc::new(tls::TlsCollector),
                Arc::new(ct::CtCollector),
                Arc::new(ip::IpCollector),
                Arc::new(repository::RepositoryCollector),
                Arc::new(file::FileCollector),
            ],
        }
    }

    pub fn all(&self) -> &[Arc<dyn Collector>] {
        &self.collectors
    }

    pub fn find(&self, id: &str) -> Option<Arc<dyn Collector>> {
        let wanted = id.trim().to_ascii_lowercase();
        self.collectors
            .iter()
            .find(|collector| {
                collector.id() == wanted || collector.label().eq_ignore_ascii_case(&wanted)
            })
            .cloned()
    }

    /// Collectors that accept the kind and whose required programs are installed.
    pub fn for_target(&self, kind: TargetKind, tools: &ToolRegistry) -> Vec<Arc<dyn Collector>> {
        self.collectors
            .iter()
            .filter(|collector| collector.accepts(kind))
            .filter(|collector| {
                collector
                    .requires()
                    .iter()
                    .all(|program| tools.is_installed(program))
            })
            .cloned()
            .collect()
    }

    /// The collectors "run all" uses for a target kind.
    pub fn defaults_for(&self, kind: TargetKind, tools: &ToolRegistry) -> Vec<Arc<dyn Collector>> {
        self.for_target(kind, tools)
            .into_iter()
            .filter(|collector| collector.runs_by_default())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolsConfig;

    #[test]
    fn domain_targets_get_the_core_collectors() {
        let registry = CollectorRegistry::builtin();
        let tools = ToolRegistry::detect(&ToolsConfig::default());
        let actual: Vec<&str> = registry
            .defaults_for(TargetKind::Domain, &tools)
            .iter()
            .map(|collector| collector.id())
            .collect();
        assert_eq!(actual, vec!["dns", "rdap", "http", "tls", "ct"]);
    }

    #[test]
    fn collectors_are_found_by_id_or_label() {
        let registry = CollectorRegistry::builtin();
        assert_eq!(
            registry.find("RDAP").map(|collector| collector.id()),
            Some("rdap")
        );
        assert!(registry.find("nope").is_none());
    }

    #[test]
    fn built_in_collectors_are_passive() {
        let registry = CollectorRegistry::builtin();
        assert!(
            registry
                .all()
                .iter()
                .all(|collector| collector.activity() == Activity::Passive)
        );
    }
}
