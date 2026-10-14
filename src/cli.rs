//! Command-line interface. `os-int` with no subcommand launches the TUI.

use std::path::PathBuf;
use std::sync::Arc;

use crate::app::App;
use crate::collectors::{CollectorContext, CollectorRegistry, CollectorSettings, NetContext};
use crate::config::Config;
use crate::files::InvestigationFiles;
use crate::ingest::{
    IngestFormat, IngestOptions, ObservableMapping, guess_format, ingest, read_ingest_file,
};
use crate::jobs::JobContext;
use crate::model::{
    JobRecord, JobStatus, MAX_CONCURRENT_PLAYBOOK_JOBS, PageKind, Target, TargetKind, Workspace,
    now,
};
use crate::paths::AppPaths;
use crate::playbook::{
    ExecuteOptions, bind_resolve_plan, bundled_ids, execute, load_playbook, plan_playbook,
};
use crate::storage::{ApplyContext, NewPage, Store};
use crate::tools::ToolRegistry;
use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "os-int",
    version,
    about = "Open-source intelligence investigation workbench for the terminal",
    long_about = "\
os-int keeps an investigation in the terminal: pages, public records, notes, and a local agent CLI.

With no subcommand it opens the workbench on the last investigation, or the welcome screen when you have none. `os-int help <command>` prints one command. The user guide is docs/guide.md."
)]
pub struct Cli {
    /// Path to config.toml. Overrides OS_INT_CONFIG, then the platform config directory.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Data directory. Overrides OS_INT_DATA_DIR, then storage.data_dir, then the platform default.
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// Write debug logs to the daily log file. Does not print logs into the workbench.
    #[arg(long, global = true)]
    pub debug: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open an investigation in the workbench.
    ///
    /// Matches the name, the UUID, or a unique UUID prefix. Stays in the terminal UI until you quit.
    Open {
        /// Investigation name, UUID, or unique UUID prefix.
        name: String,
    },
    /// Create an investigation and open it in the workbench.
    ///
    /// Stays in the terminal UI until you quit. To create a case and return to the shell, use
    /// `collect --investigation`.
    New {
        /// Name of the case. Must be unique.
        name: String,
        /// Description. Markdown reports use this as the executive summary.
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List investigations.
    ///
    /// Prints the name, an id prefix, and counts of pages, sources, findings, and entities.
    /// The most recently updated case is first.
    List,
    /// Export an investigation as a portable directory.
    ///
    /// Writes `investigation.osint.json` plus a `files/` tree of snapshots, reports, and other
    /// artifacts. Paths inside the manifest are relative.
    Export {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Directory (archive) or file (GeoJSON) to create.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// `archive` (default) writes `investigation.osint.json`. `geojson` writes a FeatureCollection.
        #[arg(long, default_value = "archive")]
        format: String,
    },
    /// Write GeoJSON for entities that have a country, prefix, or coordinates.
    Geojson {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// File to write. Defaults to `<sanitized-name>.geojson`.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print entity pairs that share an Event, a source, or a time window.
    Cooccur {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Window in hours for instants (default 24).
        #[arg(long)]
        window: Option<u64>,
    },
    /// Import a previously exported investigation.
    ///
    /// Refuses an archive whose investigation id is already in this data directory unless
    /// `--rename` is set. A colliding name is adjusted so it stays unique.
    Import {
        /// Export directory, or the `investigation.osint.json` file inside it.
        path: PathBuf,
        /// Assign a new id when an investigation with the same id already exists.
        #[arg(long)]
        rename: bool,
    },
    /// Run passive collectors and store the results.
    ///
    /// The first argument is a target kind (`domain`, `ip`, `url`, `email`, `asn`, `repository`,
    /// `file`, `username`) or a value os-int can guess (`example.com`). `--collector dns` runs
    /// one collector. Without it, the default set for that kind runs: a domain runs dns, rdap,
    /// http, tls, ct, and subdomain; an email runs dns, ct, and email; a username runs username;
    /// an IP runs rdap and ip.
    ///
    /// `--investigation` selects the case. A new name is created. A UUID that does not exist is
    /// an error. With no investigation flag, the most recently updated case is used, or one is
    /// created and named after the target.
    Collect {
        /// Target kind, or a value to guess from when `target` is omitted.
        kind: String,
        /// Target value. Omit when `kind` is itself a value such as `example.com`.
        target: Option<String>,
        /// Case to write into. Created when the name is new.
        #[arg(short, long)]
        investigation: Option<String>,
        /// One collector id (`dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, `file`, `email`, `username`, `subdomain`, or a configured external id).
        #[arg(short, long)]
        collector: Option<String>,
    },
    /// Import STIX, MISP, CSV/JSON observables, or an Evidence Batch into a case.
    ///
    /// The file is parsed, then applied through the same transaction as a collector. Re-ingest
    /// of identical content is idempotent (duplicate sources are marked seen).
    Ingest {
        /// Path to the STIX, MISP, CSV, JSON, or Evidence Batch file.
        path: PathBuf,
        /// `stix`, `misp`, `csv`, `json`, or `batch`. Guessed from the file name when omitted.
        #[arg(short, long)]
        format: Option<String>,
        /// Case to write into. Created when the name is new.
        #[arg(short, long)]
        investigation: Option<String>,
        /// Default entity kind for a single-column CSV (`domain`, `ip`, …).
        #[arg(long)]
        kind: Option<String>,
        /// Column mapping, e.g. `kind=type,value=indicator`.
        #[arg(long)]
        map: Option<String>,
    },
    /// Write a Markdown or JSON report.
    ///
    /// `json` writes JSON. Any other format value writes Markdown. Without `--output`, the file
    /// is saved under the investigation's `reports/` directory and the path is printed.
    Report {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// `markdown` (default) or `json`.
        #[arg(short, long, default_value = "markdown")]
        format: String,
        /// Copy the report to this path as well as storing it in the investigation.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run, list, or validate a playbook.
    ///
    /// Bundled ids: `domain-infra`, `mail-policy`, `cert-estate`, `repo-surface`.
    /// A YAML file is also accepted. Finding templates and merges stay pending until accept.
    /// Active tools are refused.
    Playbook {
        #[command(subcommand)]
        command: PlaybookCommand,
    },
    /// Re-run a playbook's collect steps (watch set). Dry-run prints planned Actions.
    ///
    /// `--once` runs a single pass. `--interval` loops until Ctrl+C. Automations propose
    /// on changes; they do not silently write findings.
    Watch {
        /// Bundled playbook id or YAML file.
        #[arg(long)]
        playbook: String,
        /// Case name or UUID. Created on run when the name is new; dry-run requires it to exist.
        #[arg(short, long)]
        investigation: Option<String>,
        /// Seed value of the playbook's type. Required when the case has no matching page.
        #[arg(long)]
        target: Option<String>,
        /// Print planned Actions without applying.
        #[arg(long)]
        dry_run: bool,
        /// Run one pass and exit (deterministic; used by tests).
        #[arg(long)]
        once: bool,
        /// Seconds between passes when looping (ignored with `--once` or `--dry-run`).
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Check the local environment.
    ///
    /// Reports the database, config file, log directory, proxy or direct network, and whether
    /// `claude`, `codex`, `dig`, `whois`, `git`, `gh`, `exiftool`, `curl`, `openssl`,
    /// `subfinder`, and `amass` are on PATH. A missing program is optional. A warning, usually
    /// from the database integrity check, makes this command exit non-zero.
    Doctor,
    /// Print the resolved configuration path and data directory.
    ///
    /// Also prints the database path and the log directory after flags, environment variables,
    /// and `storage.data_dir` have been applied.
    Config,
}

#[derive(Debug, Subcommand)]
pub enum PlaybookCommand {
    /// List bundled playbooks.
    List,
    /// Parse a playbook file or bundled id; print errors without writing.
    Validate {
        /// Bundled id or YAML file.
        file: String,
    },
    /// Run a playbook against a seed in an investigation.
    Run {
        /// Bundled id (`domain-infra`) or a YAML file.
        file: String,
        /// Case name or UUID. Created on run when the name is new; dry-run requires it to exist.
        #[arg(short, long)]
        investigation: Option<String>,
        /// Seed value of the playbook's type. Required when the case has no matching page.
        #[arg(long)]
        target: Option<String>,
        /// Print planned Actions without applying.
        #[arg(long)]
        dry_run: bool,
    },
}

struct Runtime {
    config: Config,
    paths: AppPaths,
    store: Store,
    _log: WorkerGuard,
}

impl Runtime {
    fn load(cli: &Cli) -> Result<Self> {
        let config_path = AppPaths::resolve_config_file(cli.config.as_deref())?;
        let loaded = Config::load_or_create(&config_path)?;
        let data_dir = AppPaths::resolve_data_dir(
            cli.data_dir.as_deref(),
            loaded.config.storage.data_dir.as_deref(),
        )?;
        let paths = AppPaths::new(loaded.path, data_dir);
        paths.ensure_dirs()?;
        let log =
            crate::logging::init(&paths, cli.debug).wrap_err("could not initialize logging")?;
        let store = Store::open(&paths.database())?;
        if loaded.created {
            tracing::info!(path = %paths.config_file.display(), "wrote default config");
        }
        for warning in loaded.config.warnings() {
            tracing::warn!("{warning}");
        }
        Ok(Self {
            config: loaded.config,
            paths,
            store,
            _log: log,
        })
    }

    fn app(self) -> App {
        App::new(self.config, self.paths, self.store)
    }
}

/// User-facing CLI error without color-eyre `Location:` noise.
fn user_err(message: impl std::fmt::Display) -> color_eyre::eyre::Report {
    color_eyre::eyre::Report::msg(message.to_string())
}

/// Display-only rendering for CLI failures (no `Location:` section).
pub fn format_cli_error(error: &color_eyre::eyre::Report) -> String {
    let mut lines = Vec::new();
    for (index, cause) in error.chain().enumerate() {
        if index == 0 {
            lines.push(cause.to_string());
        } else {
            lines.push(format!("  {cause}"));
        }
    }
    lines.join("\n")
}

/// Parses argv and either launches the TUI or runs a one-shot command.
pub async fn run() -> Result<()> {
    run_with(Cli::parse()).await
}

/// Entry used by tests that parse argv themselves.
pub async fn run_with(cli: Cli) -> Result<()> {
    match cli.command {
        None => launch_tui(&cli, None).await,
        Some(Command::Open { ref name }) => {
            let name = name.clone();
            launch_tui(&cli, Some(name)).await
        }
        Some(Command::New {
            ref name,
            ref description,
        }) => {
            let runtime = Runtime::load(&cli)?;
            let investigation = runtime
                .store
                .create_investigation(name, description.as_deref().unwrap_or(""))?;
            InvestigationFiles::new(&runtime.paths, investigation.id).ensure()?;
            println!("Created {} ({})", investigation.name, investigation.id);
            drop(runtime);
            launch_tui(&cli, Some(investigation.id.to_string())).await
        }
        Some(Command::List) => list(&cli),
        Some(Command::Export {
            ref investigation,
            ref output,
            ref format,
        }) => export(&cli, investigation, output.as_deref(), format),
        Some(Command::Geojson {
            ref investigation,
            ref output,
        }) => geojson(&cli, investigation, output.as_deref()),
        Some(Command::Cooccur {
            ref investigation,
            window,
        }) => cooccur(&cli, investigation, window),
        Some(Command::Import { ref path, rename }) => import(&cli, path, rename),
        Some(Command::Collect {
            ref kind,
            ref target,
            ref investigation,
            ref collector,
        }) => {
            collect(
                &cli,
                kind,
                target.as_deref(),
                investigation.as_deref(),
                collector.as_deref(),
            )
            .await
        }
        Some(Command::Ingest {
            ref path,
            ref format,
            ref investigation,
            ref kind,
            ref map,
        }) => ingest_file(
            &cli,
            path,
            format.as_deref(),
            investigation.as_deref(),
            kind.as_deref(),
            map.as_deref(),
        ),
        Some(Command::Report {
            ref investigation,
            ref format,
            ref output,
        }) => report(&cli, investigation, format, output.as_deref()),
        Some(Command::Playbook { ref command }) => playbook_command(&cli, command).await,
        Some(Command::Watch {
            ref playbook,
            ref investigation,
            ref target,
            dry_run,
            once,
            interval,
        }) => {
            watch_command(
                &cli,
                playbook,
                investigation.as_deref(),
                target.as_deref(),
                dry_run,
                once,
                interval,
            )
            .await
        }
        Some(Command::Doctor) => doctor(&cli),
        Some(Command::Config) => show_config(&cli),
    }
}

async fn launch_tui(cli: &Cli, open: Option<String>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let mut app = runtime.app();
    if let Some(query) = open {
        let investigation = app.store.require_investigation(&query)?;
        app.open_investigation(investigation.id)?;
    } else {
        app.open_last_or_welcome()?;
    }
    crate::tui::run(&mut app).await
}

fn list(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigations = runtime.store.list_investigations()?;
    if investigations.is_empty() {
        println!("No investigations. Run `os-int new \"Example Corp\"` or launch `os-int`.");
        return Ok(());
    }
    println!(
        "{:<28} {:<8} {:>5} {:>7} {:>8} {:>8}",
        "NAME", "ID", "PAGES", "SOURCES", "FINDINGS", "ENTITIES"
    );
    for summary in investigations {
        let id = summary.investigation.id.simple().to_string();
        println!(
            "{:<28} {:<8} {:>5} {:>7} {:>8} {:>8}",
            truncate(&summary.investigation.name, 28),
            &id[..8.min(id.len())],
            summary.page_count,
            summary.source_count,
            summary.finding_count,
            summary.entity_count
        );
    }
    Ok(())
}

fn export(cli: &Cli, query: &str, output: Option<&std::path::Path>, format: &str) -> Result<()> {
    match format.trim().to_ascii_lowercase().as_str() {
        "archive" | "osint" | "" => export_archive(cli, query, output),
        "geojson" | "geo" => geojson(cli, query, output),
        other => Err(user_err(format!(
            "unknown export format {other:?}; use archive or geojson"
        ))),
    }
}

fn export_archive(cli: &Cli, query: &str, output: Option<&std::path::Path>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(crate::exchange::suggested_export_name(&data)),
    };
    let files = runtime.paths.investigation_dir(investigation.id);
    let manifest = crate::exchange::export_investigation(
        &runtime.store,
        investigation.id,
        &files,
        &destination,
    )?;
    println!(
        "Exported {} to {}",
        investigation.name,
        manifest.parent().unwrap_or(&destination).display()
    );
    Ok(())
}

fn geojson(cli: &Cli, query: &str, output: Option<&std::path::Path>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let collection = crate::model::geojson_collection(&data);
    let json = serde_json::to_string_pretty(&collection).wrap_err("could not serialize GeoJSON")?;
    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(format!(
            "{}.geojson",
            crate::paths::sanitize_filename(&investigation.name)
        )),
    };
    crate::files::write_atomically(&destination, json.as_bytes())
        .wrap_err_with(|| format!("could not write {}", destination.display()))?;
    println!(
        "Wrote {} GeoJSON feature(s) to {}",
        collection.features.len(),
        destination.display()
    );
    Ok(())
}

fn cooccur(cli: &Cli, query: &str, window_hours: Option<u64>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let window = window_hours.map(|hours| chrono::Duration::hours(hours as i64));
    let pairs = crate::model::co_occur(
        &data,
        &crate::model::CoOccurrenceQuery {
            entity_ids: Vec::new(),
            window,
        },
    );
    if pairs.is_empty() {
        println!("No co-occurring pairs.");
        return Ok(());
    }
    let workspace = crate::model::Workspace::new(data);
    println!("{:<14} {:<8} {:<8} EVIDENCE", "KIND", "LEFT", "RIGHT");
    for pair in pairs {
        let left = workspace
            .ref_of(pair.left)
            .map(|reference| reference.to_string())
            .unwrap_or_else(|| pair.left.to_string());
        let right = workspace
            .ref_of(pair.right)
            .map(|reference| reference.to_string())
            .unwrap_or_else(|| pair.right.to_string());
        println!(
            "{:<14} {left:<8} {right:<8} {}",
            pair.kind.as_str(),
            pair.evidence
        );
    }
    Ok(())
}

fn import(cli: &Cli, path: &std::path::Path, rename: bool) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let data =
        crate::exchange::import_investigation(&mut runtime.store, path, &runtime.paths, rename)?;
    println!(
        "Imported {} ({})",
        data.investigation.name, data.investigation.id
    );
    Ok(())
}

async fn collect(
    cli: &Cli,
    kind_or_target: &str,
    target_arg: Option<&str>,
    investigation_query: Option<&str>,
    collector_id: Option<&str>,
) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let target = parse_target(kind_or_target, target_arg)?;
    let investigation = match investigation_query {
        Some(query) => match runtime.store.find_investigation(query)? {
            Some(found) => found,
            None if uuid::Uuid::parse_str(query).is_ok() => {
                return Err(user_err(format!(
                    "no investigation named {query:?}; run `os-int list` to see existing investigations"
                )));
            }
            None => {
                let created = runtime.store.create_investigation(query, "")?;
                InvestigationFiles::new(&runtime.paths, created.id).ensure()?;
                eprintln!("Created investigation {}", created.name);
                created
            }
        },
        None => match runtime.store.list_investigations()?.into_iter().next() {
            Some(summary) => summary.investigation,
            None => runtime.store.create_investigation(&target.value(), "")?,
        },
    };
    let files = InvestigationFiles::new(&runtime.paths, investigation.id);
    files.ensure()?;
    let page_kind = PageKind::from_target_kind(target.kind());
    let existing = runtime.store.load_investigation(investigation.id)?;
    let page = existing
        .pages
        .iter()
        .find(|page| page.target.as_deref() == Some(target.value().as_str()))
        .cloned();
    let page = match page {
        Some(page) => page,
        None => runtime.store.create_page(NewPage::new(
            investigation.id,
            &target.value(),
            page_kind,
            Some(&target.value()),
        ))?,
    };
    let tools = Arc::new(ToolRegistry::detect(&runtime.config.tools));
    let net = Arc::new(NetContext::new(&runtime.config.network).map_err(user_err)?);
    let registry = CollectorRegistry::from_config(&runtime.config);
    let collectors = if let Some(id) = collector_id {
        vec![
            registry
                .find(id)
                .ok_or_else(|| user_err(format!("unknown collector {id}")))?,
        ]
    } else {
        registry.defaults_for(target.kind(), &tools)
    };
    if collectors.is_empty() {
        return Err(user_err(format!(
            "no collectors available for {}",
            target.kind()
        )));
    }
    println!(
        "Investigation {}  page {} ({})",
        investigation.name,
        page.title,
        page.kind.label()
    );
    let settings = CollectorSettings::from_config(&runtime.config);
    for collector in collectors {
        let job = JobContext::detached(true);
        let context = CollectorContext {
            net: Arc::clone(&net),
            files: files.clone(),
            tools: Arc::clone(&tools),
            cache_dir: runtime.paths.cache_dir(),
            settings: settings.clone(),
            job,
        };
        print!("{:<8} ", collector.label());
        match collector.collect(&target, &context).await {
            Ok(output) => {
                let summary = runtime.store.apply_batch(
                    ApplyContext {
                        investigation_id: investigation.id,
                        page_id: Some(page.id),
                        job_id: None,
                    },
                    &output.batch,
                )?;
                let text = if output.batch.summary.is_empty() {
                    summary.describe()
                } else {
                    output.batch.summary
                };
                println!("✓  {text}");
                for warning in output.warnings {
                    println!("         ! {warning}");
                }
            }
            Err(error) => println!("✗  {error}"),
        }
    }
    Ok(())
}

fn ingest_file(
    cli: &Cli,
    path: &std::path::Path,
    format: Option<&str>,
    investigation_query: Option<&str>,
    kind: Option<&str>,
    map: Option<&str>,
) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let format = match format {
        Some(value) => IngestFormat::parse(value)
            .ok_or_else(|| user_err(format!("unknown ingest format {value:?}")))?,
        None => guess_format(path).ok_or_else(|| {
            user_err(format!(
                "could not guess ingest format from {}; pass --format stix|misp|csv|json|batch",
                path.display()
            ))
        })?,
    };
    let max_bytes = runtime.config.collectors.max_ingest_bytes;
    let bytes = read_ingest_file(path, max_bytes).map_err(user_err)?;
    let mut mapping = map
        .map(ObservableMapping::parse)
        .transpose()
        .map_err(user_err)?
        .unwrap_or_default();
    if let Some(kind) = kind {
        mapping.default_kind = Some(crate::model::EntityKind::parse(kind));
    }
    let origin = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ingest")
        .to_string();
    let options = IngestOptions {
        mapping,
        origin: origin.clone(),
    };
    let report = ingest(format, &bytes, &options).map_err(user_err)?;
    let investigation = match investigation_query {
        Some(query) => match runtime.store.find_investigation(query)? {
            Some(found) => found,
            None if uuid::Uuid::parse_str(query).is_ok() => {
                return Err(user_err(format!(
                    "no investigation named {query:?}; run `os-int list` to see existing investigations"
                )));
            }
            None => {
                let created = runtime.store.create_investigation(query, "")?;
                InvestigationFiles::new(&runtime.paths, created.id).ensure()?;
                eprintln!("Created investigation {}", created.name);
                created
            }
        },
        None => match runtime.store.list_investigations()?.into_iter().next() {
            Some(summary) => summary.investigation,
            None => runtime
                .store
                .create_investigation(&format!("ingest-{origin}"), "")?,
        },
    };
    InvestigationFiles::new(&runtime.paths, investigation.id).ensure()?;
    let summary = runtime.store.apply_batch(
        ApplyContext {
            investigation_id: investigation.id,
            page_id: None,
            job_id: None,
        },
        &report.batch,
    )?;
    println!(
        "Ingested {} into {} — {}",
        path.display(),
        investigation.name,
        if report.batch.summary.is_empty() {
            summary.describe()
        } else {
            report.batch.summary.clone()
        }
    );
    for warning in report.warnings {
        println!("  ! {warning}");
    }
    Ok(())
}

fn report(cli: &Cli, query: &str, format: &str, output: Option<&std::path::Path>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let workspace = Workspace::new(data);
    let files = InvestigationFiles::new(&runtime.paths, investigation.id);
    files.ensure()?;
    let blob = crate::report::write_report(&workspace, &files, format).map_err(user_err)?;
    let absolute = files
        .resolve(&blob.relative_path)
        .ok_or_else(|| user_err("unsafe report path"))?;
    if let Some(destination) = output {
        std::fs::copy(&absolute, destination)
            .wrap_err_with(|| format!("could not copy report to {}", destination.display()))?;
        println!("{}", destination.display());
    } else {
        println!("{}", absolute.display());
    }
    Ok(())
}

async fn playbook_command(cli: &Cli, command: &PlaybookCommand) -> Result<()> {
    match command {
        PlaybookCommand::List => {
            println!("BUNDLED PLAYBOOKS");
            for id in bundled_ids() {
                let playbook = crate::playbook::bundled(id).expect("bundled parses");
                println!("{:<16} {}", playbook.id, playbook.name);
                if !playbook.description.is_empty() {
                    println!("                 {}", playbook.description.trim());
                }
            }
            println!(
                "\nRun: os-int playbook run domain-infra --investigation NAME --target example.com"
            );
            Ok(())
        }
        PlaybookCommand::Validate { file } => {
            let playbook = load_playbook(file).map_err(user_err)?;
            println!(
                "ok  {} ({})  seed={}  {} step(s)",
                playbook.id,
                playbook.name,
                playbook.seed.kind,
                playbook.steps.len()
            );
            Ok(())
        }
        PlaybookCommand::Run {
            file,
            investigation,
            target,
            dry_run,
        } => {
            run_playbook_cli(
                cli,
                PlaybookCliOpts {
                    spec: file,
                    investigation: investigation.as_deref(),
                    target: target.as_deref(),
                    dry_run: *dry_run,
                    automation: false,
                    interval_seconds: None,
                },
            )
            .await
        }
    }
}

struct CtrlCGuard {
    token: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl CtrlCGuard {
    fn spawn() -> Self {
        let token = CancellationToken::new();
        let listener = token.clone();
        let task = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                listener.cancel();
            }
        });
        Self { token, task }
    }
}

impl Drop for CtrlCGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn watch_command(
    cli: &Cli,
    playbook_spec: &str,
    investigation: Option<&str>,
    target: Option<&str>,
    dry_run: bool,
    once: bool,
    interval: Option<u64>,
) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let guard = CtrlCGuard::spawn();
    let cancel = guard.token.clone();
    loop {
        if cancel.is_cancelled() {
            eprintln!("watch cancelled");
            return Ok(());
        }
        match run_playbook_on(
            &mut runtime,
            PlaybookCliOpts {
                spec: playbook_spec,
                investigation,
                target,
                dry_run,
                automation: true,
                interval_seconds: interval.map(|seconds| seconds as i64),
            },
            Some(cancel.clone()),
        )
        .await
        {
            Ok(()) => {}
            Err(error) if error.to_string() == "cancelled" => {
                eprintln!("watch cancelled");
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        let Some(seconds) = interval.filter(|_| !dry_run && !once) else {
            return Ok(());
        };
        tokio::select! {
            _ = cancel.cancelled() => {
                eprintln!("watch cancelled");
                return Ok(());
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(seconds.max(1))) => {}
        }
    }
}

struct PlaybookCliOpts<'a> {
    spec: &'a str,
    investigation: Option<&'a str>,
    target: Option<&'a str>,
    dry_run: bool,
    automation: bool,
    interval_seconds: Option<i64>,
}

async fn run_playbook_cli(cli: &Cli, opts: PlaybookCliOpts<'_>) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    run_playbook_on(&mut runtime, opts, None).await
}

async fn run_playbook_on(
    runtime: &mut Runtime,
    opts: PlaybookCliOpts<'_>,
    cancel: Option<CancellationToken>,
) -> Result<()> {
    let PlaybookCliOpts {
        spec,
        investigation: investigation_query,
        target: target_arg,
        dry_run,
        automation,
        interval_seconds,
    } = opts;
    let playbook = load_playbook(spec).map_err(user_err)?;
    let investigation = resolve_investigation(
        runtime,
        investigation_query,
        &format!("{}-{}", playbook.id, playbook.seed.kind),
        !dry_run,
    )?;
    let seed = resolve_playbook_seed(runtime, investigation.id, &playbook, target_arg)?;
    let tools = Arc::new(ToolRegistry::detect(&runtime.config.tools));
    let owned_guard = cancel.is_none().then(CtrlCGuard::spawn);
    let cancel = cancel.unwrap_or_else(|| owned_guard.as_ref().expect("owned guard").token.clone());
    let job = JobContext::with_cancel(true, cancel);
    if dry_run {
        let mut plan =
            plan_playbook(&playbook, &seed, &tools, automation, &job).map_err(user_err)?;
        if let Ok(data) = runtime.store.load_investigation(investigation.id) {
            bind_resolve_plan(&mut plan, &data, &playbook);
        }
        println!("{}", plan.describe());
        return Ok(());
    }
    InvestigationFiles::new(&runtime.paths, investigation.id).ensure()?;
    refuse_if_playbook_busy(&runtime.store)?;
    let files = InvestigationFiles::new(&runtime.paths, investigation.id);
    let net = Arc::new(NetContext::new(&runtime.config.network).map_err(user_err)?);
    let registry = CollectorRegistry::from_config(&runtime.config);
    let settings = CollectorSettings::from_config(&runtime.config);
    let page = existing_or_create_page(runtime, investigation.id, &seed)?;
    let previous = runtime
        .store
        .find_automation(investigation.id, &playbook.id, &seed.value())?
        .and_then(|row| row.last_digest);
    let timestamp = now();
    let mut record = JobRecord {
        id: Uuid::new_v4(),
        investigation_id: Some(investigation.id),
        seq: 0,
        kind: format!("playbook:{}", playbook.id),
        label: playbook.name.clone(),
        description: format!("{} {}", playbook.id, seed.value()),
        status: JobStatus::Running,
        program: None,
        args: Vec::new(),
        created_at: timestamp,
        started_at: Some(timestamp),
        finished_at: None,
        progress: None,
        log: String::new(),
        stderr: String::new(),
        result: None,
        error: None,
        page_id: Some(page.id),
    };
    runtime.store.insert_job(&mut record)?;
    let context = CollectorContext {
        net,
        files,
        tools: Arc::clone(&tools),
        cache_dir: runtime.paths.cache_dir(),
        settings,
        job,
    };
    let result = execute(
        &mut runtime.store,
        &playbook,
        &seed,
        ApplyContext {
            investigation_id: investigation.id,
            page_id: Some(page.id),
            job_id: Some(record.id),
        },
        &registry,
        &context,
        ExecuteOptions {
            dry_run: false,
            automation,
            previous_digest: previous,
            interval_seconds,
        },
    )
    .await;
    record.finished_at = Some(now());
    match &result {
        Ok(output) => {
            record.status = JobStatus::Succeeded;
            record.result = Some(format!("{} pending proposal(s)", output.proposals.len()));
        }
        Err(error) if error.message == "cancelled" => {
            record.status = JobStatus::Cancelled;
            record.error = Some("cancelled".into());
        }
        Err(error) => {
            record.status = JobStatus::Failed;
            record.error = Some(error.message.clone());
        }
    }
    runtime.store.update_job(&record)?;
    let output = result.map_err(user_err)?;
    println!(
        "Playbook {} on {} {} — {}",
        playbook.id,
        seed.kind(),
        seed.value(),
        if output.batches.iter().any(|batch| !batch.summary.is_empty()) {
            output
                .batches
                .iter()
                .find(|batch| !batch.summary.is_empty())
                .map(|batch| batch.summary.clone())
                .unwrap_or_else(|| format!("{} proposal(s) pending", output.proposals.len()))
        } else {
            format!("{} proposal(s) pending", output.proposals.len())
        }
    );
    for warning in output.warnings {
        println!("  ! {warning}");
    }
    for proposal in &output.proposals {
        println!("  pending  {}", proposal.summary());
    }
    Ok(())
}

fn refuse_if_playbook_busy(store: &Store) -> Result<()> {
    if store.active_playbook_job_count()? >= MAX_CONCURRENT_PLAYBOOK_JOBS {
        return Err(user_err(
            "a playbook is already running; nested playbooks are refused",
        ));
    }
    Ok(())
}

fn resolve_investigation(
    runtime: &mut Runtime,
    query: Option<&str>,
    fallback_name: &str,
    create: bool,
) -> Result<crate::model::Investigation> {
    match query {
        Some(query) => match runtime.store.find_investigation(query)? {
            Some(found) => Ok(found),
            None if create && uuid::Uuid::parse_str(query).is_err() => {
                let created = runtime.store.create_investigation(query, "")?;
                InvestigationFiles::new(&runtime.paths, created.id).ensure()?;
                eprintln!("Created investigation {}", created.name);
                Ok(created)
            }
            None => Err(user_err(format!(
                "no investigation named {query:?}; run `os-int list` to see existing investigations"
            ))),
        },
        None => match runtime.store.list_investigations()?.into_iter().next() {
            Some(summary) => Ok(summary.investigation),
            None if create => Ok(runtime.store.create_investigation(fallback_name, "")?),
            None => Err(user_err(
                "no investigation exists; create one before --dry-run",
            )),
        },
    }
}

fn resolve_playbook_seed(
    runtime: &mut Runtime,
    investigation_id: uuid::Uuid,
    playbook: &crate::model::Playbook,
    target_arg: Option<&str>,
) -> Result<Target> {
    if let Some(value) = target_arg {
        return Target::parse(playbook.seed.kind, value).map_err(user_err);
    }
    let data = runtime.store.load_investigation(investigation_id)?;
    if let Some(page) = data.pages.iter().find(|page| {
        page.target.is_some() && PageKind::from_target_kind(playbook.seed.kind) == page.kind
    }) && let Some(target) = page.collection_target()
    {
        return Ok(target);
    }
    Err(user_err(format!(
        "playbook {} needs a {} seed; pass --target",
        playbook.id, playbook.seed.kind
    )))
}

fn existing_or_create_page(
    runtime: &mut Runtime,
    investigation_id: uuid::Uuid,
    seed: &Target,
) -> Result<crate::model::Page> {
    let data = runtime.store.load_investigation(investigation_id)?;
    if let Some(page) = data
        .pages
        .iter()
        .find(|page| page.target.as_deref() == Some(seed.value().as_str()))
    {
        return Ok(page.clone());
    }
    runtime.store.create_page(NewPage::new(
        investigation_id,
        &seed.value(),
        PageKind::from_target_kind(seed.kind()),
        Some(&seed.value()),
    ))
}

fn doctor(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let tools = ToolRegistry::detect(&runtime.config.tools);
    let checks = crate::doctor::run(
        &runtime.config,
        &runtime.paths,
        Some(&runtime.store),
        &tools,
    );
    let mut failed = false;
    for check in checks {
        println!(
            "{:<16} {:<8} {}",
            check.name,
            check.status.symbol(),
            check.detail
        );
        if check.status == crate::doctor::CheckStatus::Warn {
            failed = true;
        }
    }
    if failed {
        return Err(user_err("doctor reported warnings"));
    }
    Ok(())
}

fn show_config(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    println!("config    {}", runtime.paths.config_file.display());
    println!("data      {}", runtime.paths.data_dir.display());
    println!("database  {}", runtime.paths.database().display());
    println!("logs      {}", runtime.paths.log_dir().display());
    Ok(())
}

fn parse_target(kind_or_target: &str, target: Option<&str>) -> Result<Target> {
    if let Some(value) = target {
        let kind = TargetKind::parse(kind_or_target)
            .ok_or_else(|| user_err(format!("unknown target kind {kind_or_target:?}")))?;
        Target::parse(kind, value).map_err(user_err)
    } else {
        Target::guess(kind_or_target).map_err(user_err)
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let mut output: String = text.chars().take(width.saturating_sub(1)).collect();
        output.push('…');
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_geojson_and_cooccur_and_export_format() {
        let geojson = Cli::try_parse_from(["os-int", "geojson", "Case", "-o", "out.geojson"])
            .expect("geojson");
        assert!(matches!(geojson.command, Some(Command::Geojson { .. })));
        let cooccur =
            Cli::try_parse_from(["os-int", "cooccur", "Case", "--window", "12"]).expect("cooccur");
        assert!(matches!(
            cooccur.command,
            Some(Command::Cooccur {
                window: Some(12),
                ..
            })
        ));
        let export = Cli::try_parse_from([
            "os-int",
            "export",
            "Case",
            "--format",
            "geojson",
            "-o",
            "out.geojson",
        ])
        .expect("export");
        match export.command {
            Some(Command::Export { format, .. }) => assert_eq!(format, "geojson"),
            other => panic!("expected export {other:?}"),
        }
    }

    #[test]
    fn parses_collect_kind_and_value() {
        let actual = parse_target("domain", Some("Example.COM")).expect("target");
        assert_eq!(actual, Target::Domain("example.com".into()));
    }

    #[test]
    fn guesses_a_bare_domain() {
        let actual = parse_target("example.com", None).expect("guess");
        assert_eq!(actual, Target::Domain("example.com".into()));
    }

    #[test]
    fn user_errors_have_no_location_noise() {
        let error = user_err("no investigation named \"Never Created\"");
        let rendered = format_cli_error(&error);
        assert!(
            !rendered.contains("Location"),
            "user errors must not include color-eyre Location noise: {rendered}"
        );
        assert!(rendered.contains("Never Created"));
    }

    #[test]
    fn playbook_target_help_does_not_claim_to_create() {
        let run = Cli::command()
            .find_subcommand("playbook")
            .expect("playbook")
            .find_subcommand("run")
            .expect("run")
            .clone()
            .render_long_help()
            .to_string();
        assert!(!run.to_ascii_lowercase().contains("creates one"), "{run}");
        assert!(run.contains("Seed value of the playbook's type"), "{run}");
    }

    #[test]
    fn concurrent_playbook_job_cap_refuses_a_second_run() {
        let store = Store::open_in_memory().expect("store");
        refuse_if_playbook_busy(&store).expect("idle");
        let timestamp = now();
        let mut job = JobRecord {
            id: Uuid::new_v4(),
            investigation_id: None,
            seq: 0,
            kind: "playbook:domain-infra".into(),
            label: "Domain infrastructure".into(),
            description: "cap".into(),
            status: JobStatus::Running,
            program: None,
            args: Vec::new(),
            created_at: timestamp,
            started_at: Some(timestamp),
            finished_at: None,
            progress: None,
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id: None,
        };
        store.insert_job(&mut job).expect("insert");
        assert_eq!(store.active_playbook_job_count().expect("count"), 1);
        let error = refuse_if_playbook_busy(&store).expect_err("cap");
        let rendered = format_cli_error(&error);
        assert!(rendered.contains("already running"));
        assert!(!rendered.contains("Location"));
    }

    #[tokio::test]
    async fn dry_run_does_not_insert_a_new_investigation_name() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = directory.path().join("config.toml");
        let data = directory.path().join("data");
        let cli = Cli::try_parse_from([
            "os-int",
            "--config",
            config.to_str().expect("utf-8"),
            "--data-dir",
            data.to_str().expect("utf-8"),
            "playbook",
            "run",
            "domain-infra",
            "--investigation",
            "Never Created",
            "--target",
            "example.com",
            "--dry-run",
        ])
        .expect("parse");
        let error = run_with(cli).await.expect_err("missing case");
        let rendered = format_cli_error(&error);
        assert!(
            !rendered.contains("Location"),
            "dry-run missing-case error must be a clean user error: {rendered}"
        );
        assert!(
            error.to_string().contains("Never Created")
                || error.to_string().contains("no investigation"),
            "{error}"
        );
        let paths = AppPaths::new(config, data);
        let store = Store::open(&paths.database()).expect("store");
        assert!(
            store
                .find_investigation("Never Created")
                .expect("lookup")
                .is_none()
        );
        assert!(store.list_investigations().expect("list").is_empty());
    }
}
