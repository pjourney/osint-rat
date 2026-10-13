//! Command-line interface. `os-int` with no subcommand launches the TUI.

use std::path::PathBuf;
use std::sync::Arc;

use crate::app::App;
use crate::collectors::{CollectorContext, CollectorRegistry, CollectorSettings, NetContext};
use crate::config::Config;
use crate::files::InvestigationFiles;
use crate::jobs::JobContext;
use crate::model::{PageKind, Target, TargetKind, Workspace};
use crate::paths::AppPaths;
use crate::storage::{ApplyContext, NewPage, Store};
use crate::tools::ToolRegistry;
use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use tracing_appender::non_blocking::WorkerGuard;

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
        /// Directory to create. Defaults to `<sanitized-name>-<uuid>` in the current directory.
        #[arg(short, long)]
        output: Option<PathBuf>,
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
    /// http, tls, and ct; an IP runs rdap and ip.
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
        /// One collector id (`dns`, `rdap`, `http`, `tls`, `ct`, `ip`, `repository`, `file`).
        #[arg(short, long)]
        collector: Option<String>,
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
    /// Check the local environment.
    ///
    /// Reports the database, config file, log directory, proxy or direct network, and whether
    /// `claude`, `codex`, `dig`, `whois`, `git`, `gh`, `exiftool`, `curl`, and `openssl` are on
    /// PATH. A missing program is optional. A warning, usually from the database integrity
    /// check, makes this command exit non-zero.
    Doctor,
    /// Print the resolved configuration path and data directory.
    ///
    /// Also prints the database path and the log directory after flags, environment variables,
    /// and `storage.data_dir` have been applied.
    Config,
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

/// Parses argv and either launches the TUI or runs a one-shot command.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
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
        }) => export(&cli, investigation, output.as_deref()),
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
        Some(Command::Report {
            ref investigation,
            ref format,
            ref output,
        }) => report(&cli, investigation, format, output.as_deref()),
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

fn export(cli: &Cli, query: &str, output: Option<&std::path::Path>) -> Result<()> {
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
                return Err(eyre!(
                    "no investigation named {query:?}; run `os-int list` to see existing investigations"
                ));
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
    let net = Arc::new(NetContext::new(&runtime.config.network).map_err(|error| eyre!(error))?);
    let registry = CollectorRegistry::builtin();
    let collectors = if let Some(id) = collector_id {
        vec![
            registry
                .find(id)
                .ok_or_else(|| eyre!("unknown collector {id}"))?,
        ]
    } else {
        registry.defaults_for(target.kind(), &tools)
    };
    if collectors.is_empty() {
        bail!("no collectors available for {}", target.kind());
    }
    println!(
        "Investigation {}  page {} ({})",
        investigation.name,
        page.title,
        page.kind.label()
    );
    let settings = CollectorSettings {
        fetch_robots: runtime.config.collectors.fetch_robots,
        check_www: runtime.config.collectors.check_www,
        repository_history_depth: runtime.config.collectors.repository_history_depth,
        max_file_import_bytes: runtime.config.storage.max_file_import_bytes,
    };
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

fn report(cli: &Cli, query: &str, format: &str, output: Option<&std::path::Path>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let workspace = Workspace::new(data);
    let files = InvestigationFiles::new(&runtime.paths, investigation.id);
    files.ensure()?;
    let blob =
        crate::report::write_report(&workspace, &files, format).map_err(|error| eyre!(error))?;
    let absolute = files
        .resolve(&blob.relative_path)
        .ok_or_else(|| eyre!("unsafe report path"))?;
    if let Some(destination) = output {
        std::fs::copy(&absolute, destination)
            .wrap_err_with(|| format!("could not copy report to {}", destination.display()))?;
        println!("{}", destination.display());
    } else {
        println!("{}", absolute.display());
    }
    Ok(())
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
        bail!("doctor reported warnings");
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
            .ok_or_else(|| eyre!("unknown target kind {kind_or_target:?}"))?;
        Target::parse(kind, value).map_err(|error| eyre!(error))
    } else {
        Target::guess(kind_or_target).map_err(|error| eyre!(error))
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
    fn parses_collect_kind_and_value() {
        let actual = parse_target("domain", Some("Example.COM")).expect("target");
        assert_eq!(actual, Target::Domain("example.com".into()));
    }

    #[test]
    fn guesses_a_bare_domain() {
        let actual = parse_target("example.com", None).expect("guess");
        assert_eq!(actual, Target::Domain("example.com".into()));
    }
}
