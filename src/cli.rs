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
    AgentAction, AuditKind, DEFAULT_SEARCH_HOPS, EntityKind, JobRecord, JobStatus,
    MAX_CONCURRENT_PLAYBOOK_JOBS, Marking, PageKind, Role, Target, TargetKind, Workspace, now,
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
    /// ASCII glyphs, no colour, and motion off. Use with a screen reader or a dumb terminal.
    ///
    /// `OS_INT_PLAIN` is presence-based like `NO_COLOR`: `1`, empty, `yes`, and `on` enable it.
    /// `false`, `0`, `no`, `off`, and `n` disable it.
    #[arg(long, global = true)]
    pub plain: bool,
    /// Machine-readable JSON for list commands.
    #[arg(long, global = true)]
    pub json: bool,
    /// Confirm a write without an interactive prompt. Required when stdin is not a TTY.
    #[arg(long, global = true)]
    pub yes: bool,
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
    /// Show or set the Traffic Light Protocol marking on a case (or an object).
    Marking {
        #[command(subcommand)]
        command: MarkingCommand,
    },
    /// List, add, or change case members (`owner` / `analyst` / `reader`).
    Team {
        #[command(subcommand)]
        command: TeamCommand,
    },
    /// Print the append-only audit log for an investigation.
    Audit {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Optional kind filter (`merge`, `accept`, `export`, `playbook`, `marking`).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Print Object View JSON for an entity (`E-n`) or page (`P-n`).
    Get {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Short reference (`E-1`) or UUID.
        reference: String,
    },
    /// List visible entities as JSON (scriptable).
    Objects {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Optional entity kind filter (`domain`, `ip`, …).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Search Around a seed entity and print JSON.
    #[command(name = "search-around")]
    SearchAround {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Seed entity short reference (`E-1`), UUID, or exact value (`example.com`).
        seed: String,
        /// Hop depth (default 2).
        #[arg(long)]
        hops: Option<u32>,
        /// Optional entity kind filter.
        #[arg(long)]
        kind: Option<String>,
        /// Optional link-type filter (`resolves_to`, …).
        #[arg(long)]
        link: Option<String>,
    },
    /// Suggest, propose, or apply entity resolution. Merge is an analyst action; propose stays pending.
    Resolve {
        #[command(subcommand)]
        command: ResolveCommand,
    },
    /// Serve the localhost ontology HTTP API (read + Action propose).
    Serve {
        /// Bind address. Defaults to `[api].bind` then `127.0.0.1:8743`.
        #[arg(long)]
        bind: Option<String>,
        /// Bearer token. Defaults to `[api].token`, then `OS_INT_API_TOKEN`, then an ephemeral token.
        #[arg(long, env = "OS_INT_API_TOKEN")]
        token: Option<String>,
        /// Allow binding to a non-loopback address. Refused by default.
        #[arg(long)]
        allow_non_loopback: bool,
    },
    /// Measure load/query times for synthetic graphs (1k / 10k / 100k).
    Bench {
        /// Comma-separated entity counts. Default: `1000,10000,100000`.
        #[arg(long)]
        sizes: Option<String>,
    },
    /// Preview cockpit themes without opening the workbench.
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },
    /// Print the Case home dashboard (targets, glance, NEXT).
    Case {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
    },
    /// List or decide Review items (`X-n` refs). Accept always requires `--yes` or an interactive y.
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    /// List jobs for a case.
    Jobs {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
    },
    /// List findings and sources (Evidence board).
    Evidence {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
    },
    /// List tools. Active tools are refused, with no enable option.
    Tools,
}

#[derive(Debug, Subcommand)]
pub enum ResolveCommand {
    /// Print scored merge suggestions. Never merges.
    Suggest { investigation: String },
    /// Store a pending PROPOSE_RESOLVE Action. Does not merge.
    Propose {
        investigation: String,
        /// Entity refs or UUIDs. First is the intended canonical.
        entities: Vec<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Merge entities (analyst action). First id is canonical.
    Merge {
        investigation: String,
        entities: Vec<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Split a previously resolved bag.
    Unresolve {
        investigation: String,
        /// Canonical entity or resolution ref (`E-n` / `V-n`).
        target: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum MarkingCommand {
    /// Print the investigation marking (and object marking when `--entity` is set).
    Get {
        investigation: String,
        #[arg(long)]
        entity: Option<String>,
    },
    /// Set TLP:CLEAR / GREEN / AMBER / AMBER+STRICT / RED, or unmarked.
    Set {
        investigation: String,
        marking: String,
        #[arg(long)]
        entity: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TeamCommand {
    /// List members. Empty list means single-user owner.
    List { investigation: String },
    /// Add a member. The first add seeds the local identity as owner.
    Add {
        investigation: String,
        user_id: String,
        #[arg(long)]
        role: String,
    },
    /// Change a member's role.
    SetRole {
        investigation: String,
        user_id: String,
        #[arg(long)]
        role: String,
    },
    /// Remove a member. The last owner cannot be removed.
    Remove {
        investigation: String,
        user_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// Print pending Review items. Never writes.
    List {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
    },
    /// Accept a pending proposal (`X-n` or UUID). Requires `--yes` or an interactive y.
    Accept {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Proposal short reference (`X-1`) or full UUID. Exact match only; `X-1` does not select `X-10`.
        reference: String,
    },
    /// Reject a pending proposal (`X-n` or UUID). Requires `--yes` or an interactive y.
    Reject {
        /// Investigation name, UUID, or unique UUID prefix.
        investigation: String,
        /// Proposal short reference (`X-1`) or full UUID. Exact match only; `X-1` does not select `X-10`.
        reference: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ThemeCommand {
    /// Print role swatches, a sample strip, and colour-blind ΔE.
    Preview {
        /// Theme name (`dark`, `light`, `mono`, `phosphor`, `amber`). Default: configured / auto.
        name: Option<String>,
        /// Stack all five built-in themes.
        #[arg(long)]
        all: bool,
    },
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
        let mut store = Store::open(&paths.database())?;
        store.set_identity(crate::model::Identity::resolve(
            &loaded.config.identity.user_id,
            &loaded.config.identity.machine_name,
        ));
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
pub async fn run_with(mut cli: Cli) -> Result<()> {
    if crate::tui::env_os_int_plain() {
        cli.plain = true;
    }
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
        Some(Command::Marking { ref command }) => marking_command(&cli, command),
        Some(Command::Team { ref command }) => team_command(&cli, command),
        Some(Command::Audit {
            ref investigation,
            ref kind,
        }) => audit_command(&cli, investigation, kind.as_deref()),
        Some(Command::Get {
            ref investigation,
            ref reference,
        }) => get_object_command(&cli, investigation, reference),
        Some(Command::Objects {
            ref investigation,
            ref kind,
        }) => objects_command(&cli, investigation, kind.as_deref()),
        Some(Command::SearchAround {
            ref investigation,
            ref seed,
            hops,
            ref kind,
            ref link,
        }) => search_around_command(
            &cli,
            investigation,
            seed,
            hops,
            kind.as_deref(),
            link.as_deref(),
        ),
        Some(Command::Resolve { ref command }) => resolve_command(&cli, command),
        Some(Command::Serve {
            ref bind,
            ref token,
            allow_non_loopback,
        }) => serve_command(&cli, bind.as_deref(), token.as_deref(), allow_non_loopback).await,
        Some(Command::Bench { ref sizes }) => bench_command(&cli, sizes.as_deref()),
        Some(Command::Theme { ref command }) => theme_command(&cli, command),
        Some(Command::Case { ref investigation }) => case_command(&cli, investigation),
        Some(Command::Review { ref command }) => review_command(&cli, command),
        Some(Command::Jobs { ref investigation }) => jobs_command(&cli, investigation),
        Some(Command::Evidence { ref investigation }) => evidence_command(&cli, investigation),
        Some(Command::Tools) => tools_command(&cli),
    }
}

fn theme_command(cli: &Cli, command: &ThemeCommand) -> Result<()> {
    match command {
        ThemeCommand::Preview { name, all } => {
            let config = load_config_only(cli)?;
            let env = crate::tui::TerminalEnv::from_process();
            print!(
                "{}",
                crate::tui::render_preview(&config, &env, name.as_deref(), *all)?
            );
            Ok(())
        }
    }
}

/// Theme preview must not open the database or write a default config.
fn load_config_only(cli: &Cli) -> Result<Config> {
    let path = AppPaths::resolve_config_file(cli.config.as_deref())?;
    if path.exists() {
        Config::load(&path)
    } else {
        Ok(Config::default())
    }
}

async fn launch_tui(cli: &Cli, open: Option<String>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let mut app = runtime.app();
    if cli.plain {
        app.apply_plain_mode();
    }
    let cold_start = open.is_none();
    if let Some(query) = open {
        let investigation = app.store.require_investigation(&query)?;
        app.open_investigation(investigation.id)?;
    } else {
        app.open_last_or_welcome()?;
    }
    if cold_start && std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let splash = app.workspace.as_ref().map(|workspace| {
            (
                workspace.investigation().name.clone(),
                workspace.investigation().marking.label().to_string(),
            )
        });
        if let Some((name, marking)) = splash {
            app.motion.start_splash(&name, &marking);
        }
    }
    crate::tui::run(&mut app).await
}

fn confirm_write(cli: &Cli, prompt: &str) -> Result<bool> {
    if cli.yes {
        return Ok(true);
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(user_err(format!(
            "{prompt} Refusing to write without --yes when stdin is not a TTY."
        )));
    }
    eprint!("{prompt} [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn cli_theme(cli: &Cli) -> crate::tui::Theme {
    let mut config = load_config_only(cli).unwrap_or_default();
    let mut env = crate::tui::TerminalEnv::from_process();
    if cli.plain || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        env.no_color = true;
    }
    if cli.plain {
        config.ui.ascii = true;
    }
    crate::tui::Theme::resolve(&config, &env)
}

fn list(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigations = runtime.store.list_investigations()?;
    if cli.json {
        return print_json(&investigations);
    }
    if investigations.is_empty() {
        println!("No investigations. Run `os-int new \"Example Corp\"` or launch `os-int`.");
        return Ok(());
    }
    println!(
        "{:<28} {:<8} {:<16} {:>5} {:>7} {:>8} {:>8}",
        "NAME", "ID", "MARKING", "PAGES", "SOURCES", "FINDINGS", "ENTITIES"
    );
    for summary in investigations {
        let id = summary.investigation.id.simple().to_string();
        println!(
            "{:<28} {:<8} {:<16} {:>5} {:>7} {:>8} {:>8}",
            truncate(&summary.investigation.name, 28, cli.plain),
            &id[..8.min(id.len())],
            summary.investigation.marking.label(),
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
        "stix" | "stix2" | "stix2.1" => export_stix(cli, query, output),
        other => Err(user_err(format!(
            "unknown export format {other:?}; use archive, geojson, or stix"
        ))),
    }
}

fn export_archive(cli: &Cli, query: &str, output: Option<&std::path::Path>) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(crate::exchange::suggested_export_name(&data)),
    };
    let files = runtime.paths.investigation_dir(investigation.id);
    let manifest = crate::exchange::export_investigation(
        &mut runtime.store,
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
    let window = window_hours.map(parse_cooccur_window).transpose()?;
    let pairs = crate::model::co_occur(
        &data,
        &crate::model::CoOccurrenceQuery {
            entity_ids: Vec::new(),
            window,
        },
    );
    let workspace = crate::model::Workspace::new(data);
    let rows: Vec<serde_json::Value> = pairs
        .iter()
        .map(|pair| {
            let left = workspace
                .ref_of(pair.left)
                .map(|reference| reference.to_string())
                .unwrap_or_else(|| pair.left.to_string());
            let right = workspace
                .ref_of(pair.right)
                .map(|reference| reference.to_string())
                .unwrap_or_else(|| pair.right.to_string());
            serde_json::json!({
                "kind": pair.kind.as_str(),
                "left": left,
                "right": right,
                "left_id": pair.left,
                "right_id": pair.right,
                "evidence": pair.evidence,
            })
        })
        .collect();
    if cli.json {
        return print_json(&rows);
    }
    if pairs.is_empty() {
        println!("No co-occurring pairs.");
        return Ok(());
    }
    println!("{:<14} {:<8} {:<8} EVIDENCE", "KIND", "LEFT", "RIGHT");
    for row in &rows {
        println!(
            "{:<14} {:<8} {:<8} {}",
            row["kind"].as_str().unwrap_or(""),
            row["left"].as_str().unwrap_or(""),
            row["right"].as_str().unwrap_or(""),
            row["evidence"].as_str().unwrap_or("")
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
    let theme = cli_theme(cli);
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
                println!("{}  {text}", theme.glyphs.done);
                for warning in output.warnings {
                    println!("         ! {warning}");
                }
            }
            Err(error) => println!("{}  {error}", theme.glyphs.failed),
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
        "Ingested {} into {} - {}",
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
            let mut rows = Vec::new();
            for id in bundled_ids() {
                let Some(playbook) = crate::playbook::bundled(id) else {
                    return Err(user_err(format!("bundled playbook {id} failed to parse")));
                };
                rows.push(serde_json::json!({
                    "id": playbook.id,
                    "name": playbook.name,
                    "description": playbook.description.trim(),
                    "seed": playbook.seed.kind.as_str(),
                    "steps": playbook.steps.len(),
                }));
            }
            if cli.json {
                return print_json(&rows);
            }
            println!("BUNDLED PLAYBOOKS");
            for row in &rows {
                println!(
                    "{:<16} {}",
                    row["id"].as_str().unwrap_or(""),
                    row["name"].as_str().unwrap_or("")
                );
                let description = row["description"].as_str().unwrap_or("");
                if !description.is_empty() {
                    println!("                 {description}");
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
    let interval = interval.map(parse_watch_interval).transpose()?;
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
            _ = tokio::time::sleep(crate::config::duration_from_secs(seconds.max(1))) => {}
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
        "Playbook {} on {} {} - {}",
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

fn marking_command(cli: &Cli, command: &MarkingCommand) -> Result<()> {
    match command {
        MarkingCommand::Get {
            investigation,
            entity,
        } => {
            let runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            if let Some(query) = entity {
                let data = runtime.store.load_investigation(investigation.id)?;
                let workspace = Workspace::new(data);
                let entity_id = resolve_entity_query(&workspace, query)?;
                let entity = workspace
                    .entity(entity_id)
                    .ok_or_else(|| user_err(format!("unknown entity {query}")))?;
                if cli.json {
                    return print_json(&serde_json::json!({
                        "investigation": investigation.name,
                        "ref": entity.short_ref().to_string(),
                        "name": entity.display_name(),
                        "marking": entity.marking().label(),
                    }));
                }
                println!(
                    "{} {}  {}",
                    entity.short_ref(),
                    entity.display_name(),
                    entity.marking().label()
                );
            } else if cli.json {
                return print_json(&serde_json::json!({
                    "investigation": investigation.name,
                    "marking": investigation.marking.label(),
                }));
            } else {
                println!("{}  {}", investigation.name, investigation.marking.label());
            }
            Ok(())
        }
        MarkingCommand::Set {
            investigation,
            marking,
            entity,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let parsed = Marking::parse(marking).map_err(user_err)?;
            if let Some(query) = entity {
                let data = runtime.store.load_investigation(investigation.id)?;
                let workspace = Workspace::new(data);
                let entity_id = resolve_entity_query(&workspace, query)?;
                workspace
                    .entity(entity_id)
                    .ok_or_else(|| user_err(format!("unknown entity {query}")))?;
                runtime.store.set_entity_marking(entity_id, parsed)?;
                println!("Set object marking to {}", parsed.label());
            } else {
                runtime.store.set_marking(investigation.id, parsed)?;
                println!("Set {} to {}", investigation.name, parsed.label());
            }
            Ok(())
        }
    }
}

fn team_command(cli: &Cli, command: &TeamCommand) -> Result<()> {
    match command {
        TeamCommand::List { investigation } => {
            let runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let members = runtime.store.list_members(investigation.id)?;
            let role = runtime.store.role_for(investigation.id)?;
            if cli.json {
                return print_json(&serde_json::json!({
                    "investigation": investigation.name,
                    "identity": runtime.store.identity().user_id,
                    "role": role.as_str(),
                    "members": members,
                }));
            }
            println!(
                "{}  local identity {} ({})",
                investigation.name,
                runtime.store.identity().user_id,
                role.as_str()
            );
            if members.is_empty() {
                println!("(no members; single-user owner)");
                return Ok(());
            }
            for member in members {
                println!(
                    "  {:<16}  {:<8}  added by {}",
                    member.user_id,
                    member.role.as_str(),
                    member.added_by
                );
            }
            Ok(())
        }
        TeamCommand::Add {
            investigation,
            user_id,
            role,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let role = Role::parse(role).map_err(user_err)?;
            runtime.store.add_member(investigation.id, user_id, role)?;
            println!("Added {user_id} as {}", role.as_str());
            Ok(())
        }
        TeamCommand::SetRole {
            investigation,
            user_id,
            role,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let role = Role::parse(role).map_err(user_err)?;
            runtime
                .store
                .set_member_role(investigation.id, user_id, role)?;
            println!("Set {user_id} to {}", role.as_str());
            Ok(())
        }
        TeamCommand::Remove {
            investigation,
            user_id,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            runtime.store.remove_member(investigation.id, user_id)?;
            println!("Removed {user_id}");
            Ok(())
        }
    }
}

fn case_command(cli: &Cli, query: &str) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let mut app = runtime.app();
    if cli.plain {
        app.apply_plain_mode();
    }
    app.open_investigation(investigation.id)?;
    let home = app.case_home();
    if cli.json {
        return print_json(&serde_json::json!({
            "investigation": investigation.name,
            "empty": home.is_empty,
            "targets": home.targets.iter().map(|target| serde_json::json!({
                "kind": target.kind_tag().trim(),
                "value": target.value,
                "sources": target.source_count,
                "never_collected": target.never_collected,
                "running": target.running,
            })).collect::<Vec<_>>(),
            "entities": home.entities,
            "findings": home.findings,
            "next": home.next.iter().map(|item| serde_json::json!({
                "label": item.label,
                "hint": item.hint,
            })).collect::<Vec<_>>(),
        }));
    }
    let theme = cli_theme(cli);
    println!("CASE  {}", investigation.name);
    println!("TARGETS");
    if home.targets.is_empty() {
        println!("  {}  No targets yet. n new target", theme.glyphs.external);
    }
    for target in &home.targets {
        let glyph = target
            .provenance
            .map(|provenance| theme.glyphs.provenance(provenance).to_string())
            .unwrap_or_else(|| theme.glyphs.dash().to_string());
        println!(
            "  {} {:<22} {glyph} {}  {}",
            target.kind_tag(),
            truncate(&target.value, 22, cli.plain),
            target.source_count,
            App::case_target_status_label(target, theme.glyphs.ascii)
        );
    }
    println!("AT A GLANCE");
    println!(
        "  {} entities   {}",
        home.entities.total,
        format_glance_buckets(&home.entities, &theme)
    );
    println!(
        "  {} findings   {}",
        home.findings.total,
        format_glance_buckets(&home.findings, &theme)
    );
    println!("NEXT");
    if home.next.is_empty() {
        println!("  Case is current.");
    }
    for item in &home.next {
        println!("  {}  {}", item.label, item.hint);
    }
    Ok(())
}

fn review_command(cli: &Cli, command: &ReviewCommand) -> Result<()> {
    match command {
        ReviewCommand::List { investigation } => review_list(cli, investigation),
        ReviewCommand::Accept {
            investigation,
            reference,
        } => review_decide(cli, investigation, reference, true),
        ReviewCommand::Reject {
            investigation,
            reference,
        } => review_decide(cli, investigation, reference, false),
    }
}

fn review_list(cli: &Cli, query: &str) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let mut app = runtime.app();
    if cli.plain {
        app.apply_plain_mode();
    }
    app.open_investigation(investigation.id)?;
    app.open_review_lane();
    let items = app.listed_review_items();
    if cli.json {
        return print_json(
            &items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "title": item.title,
                        "group": item.group,
                        "kind": item.kind.as_str(),
                        "short_ref": item.short_ref,
                        "proposal_id": item.proposal_id.map(|id| id.to_string()),
                        "writable": item.is_writable(),
                        "ai": item.is_ai(),
                        "missing_citation": item.has_missing_citation(),
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    let theme = cli_theme(cli);
    if items.is_empty() {
        println!("Review is empty. Agent, playbook, and merge proposals land here.");
        return Ok(());
    }
    let sep = if theme.glyphs.ascii { "-" } else { "·" };
    println!("REVIEW  AI proposes only {sep} nothing is written until you accept");
    for item in &items {
        let mark = if item.is_writable() {
            format!("{}?", theme.glyphs.ai)
        } else {
            theme.glyphs.ai.to_string()
        };
        println!("  {mark} {}  {}", item.group, item.title);
    }
    Ok(())
}

fn review_decide(cli: &Cli, query: &str, reference: &str, accept: bool) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let mut app = runtime.app();
    if cli.plain {
        app.apply_plain_mode();
    }
    app.open_investigation(investigation.id)?;
    app.open_review_lane();
    let items = app.listed_review_items();
    let index = items
        .iter()
        .position(|item| item.matches_reference(reference));
    let Some(index) = index else {
        return Err(user_err(format!("no Review item {reference}")));
    };
    let item = &items[index];
    review_write_guard(item, accept)?;
    let prompt = review_confirm_prompt(item, accept);
    if !confirm_write(cli, &prompt)? {
        return Err(user_err("cancelled; nothing written"));
    }
    let written_as = item.written_as;
    app.selected = index;
    if accept {
        app.review_armed = true;
        app.dispatch(crate::app::Action::Accept)?;
        println!("{}", review_accepted_copy(written_as, app.config.ui.ascii));
    } else {
        app.dispatch(crate::app::Action::Reject)?;
        println!("Rejected {reference}.");
    }
    Ok(())
}

fn review_write_guard(item: &crate::app::review::ReviewItem, accept: bool) -> Result<()> {
    if accept && !item.is_writable() {
        return Err(user_err(
            "that Review row is transcript only; nothing to accept",
        ));
    }
    if accept && item.has_missing_citation() {
        return Err(user_err(
            "a citation points at a missing source; acceptance is blocked",
        ));
    }
    Ok(())
}

fn review_confirm_prompt(item: &crate::app::review::ReviewItem, accept: bool) -> String {
    if !accept {
        return format!("Reject {:?}?", item.title);
    }
    match item.written_as {
        Some(crate::model::Provenance::Agent) => format!(
            "Accept AI item {:?}? It keeps agent provenance.",
            item.title
        ),
        Some(crate::model::Provenance::Analyst) => {
            format!("Accept {:?}? It keeps analyst provenance.", item.title)
        }
        Some(other) => format!(
            "Accept {:?}? It keeps {} provenance.",
            item.title,
            other.as_str()
        ),
        None => format!("Accept {:?}?", item.title),
    }
}

fn review_accepted_copy(written_as: Option<crate::model::Provenance>, ascii: bool) -> String {
    let glyph = if ascii { "*" } else { "◆" };
    match written_as {
        Some(crate::model::Provenance::Agent) => {
            format!("Accepted. AI items keep {glyph} agent provenance.")
        }
        Some(crate::model::Provenance::Analyst) => "Accepted. Analyst provenance kept.".into(),
        Some(other) => format!("Accepted. {} provenance kept.", other.as_str()),
        None => "Accepted.".into(),
    }
}

fn jobs_command(cli: &Cli, query: &str) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    if cli.json {
        return print_json(&data.jobs);
    }
    let theme = cli_theme(cli);
    if data.jobs.is_empty() {
        println!("No jobs.");
        return Ok(());
    }
    for job in &data.jobs {
        let mark = job.status.symbol(theme.glyphs.ascii);
        println!(
            "{} {}  {}  {}",
            mark,
            job.short_ref(),
            job.label,
            job.status.as_str()
        );
    }
    Ok(())
}

fn evidence_command(cli: &Cli, query: &str) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    if cli.json {
        return print_json(&serde_json::json!({
            "findings": data.findings,
            "sources": data.sources.iter().map(|source| serde_json::json!({
                "ref": source.short_ref().to_string(),
                "title": source.title,
                "provenance": source.provenance.as_str(),
            })).collect::<Vec<_>>(),
        }));
    }
    let theme = cli_theme(cli);
    println!("EVIDENCE");
    for finding in &data.findings {
        println!(
            "  F  {}  {}  {}",
            theme.glyphs.provenance(finding.provenance),
            finding.short_ref(),
            finding.title
        );
    }
    for source in &data.sources {
        println!(
            "  S  {}  {}  {}",
            theme.glyphs.provenance(source.provenance),
            source.short_ref(),
            source.title
        );
    }
    if data.findings.is_empty() && data.sources.is_empty() {
        println!("  No evidence yet.");
    }
    Ok(())
}

fn tools_command(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let app = runtime.app();
    let theme = cli_theme(cli);
    let rows: Vec<serde_json::Value> = app
        .tools
        .tools()
        .iter()
        .map(|entry| {
            let refused = entry.definition.activity == crate::tools::Activity::Active;
            serde_json::json!({
                "name": entry.definition.name,
                "activity": entry.definition.activity.as_str(),
                "refused": refused,
                "available": entry.is_available(),
            })
        })
        .collect();
    if cli.json {
        return print_json(&rows);
    }
    for entry in app.tools.tools() {
        let refused = entry.definition.activity == crate::tools::Activity::Active;
        let mark = if refused {
            theme.glyphs.refused
        } else if entry.is_available() {
            theme.glyphs.done
        } else {
            theme.glyphs.missing()
        };
        let status = if refused {
            format!(
                "active  refused: {}",
                entry
                    .problem
                    .as_deref()
                    .unwrap_or("os-int observes public information only")
            )
        } else if entry.is_available() {
            "passive".into()
        } else {
            "missing".into()
        };
        println!("{mark}  {:<16}  {status}", entry.definition.name);
    }
    Ok(())
}

fn audit_command(cli: &Cli, query: &str, kind: Option<&str>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let filter = kind.map(AuditKind::parse).transpose().map_err(user_err)?;
    let entries = runtime.store.list_audit(investigation.id)?;
    let filtered: Vec<_> = entries
        .into_iter()
        .filter(|entry| filter.is_none_or(|kind| entry.kind == kind))
        .collect();
    if cli.json {
        return print_json(&filtered);
    }
    if filtered.is_empty() {
        println!("No audit events.");
        return Ok(());
    }
    for entry in filtered {
        println!(
            "{}  {}  {}@{}  {}",
            entry.short_ref(),
            entry.occurred_at.to_rfc3339(),
            entry.actor,
            entry.machine,
            entry.summary
        );
    }
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).wrap_err("could not serialize JSON")?
    );
    Ok(())
}

fn export_stix(cli: &Cli, query: &str, output: Option<&std::path::Path>) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let bundle = crate::ingest::export_stix_bundle(&data);
    let json = serde_json::to_string_pretty(&bundle).wrap_err("could not serialize STIX")?;
    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(format!(
            "{}-stix.json",
            crate::paths::sanitize_filename(&investigation.name)
        )),
    };
    crate::files::write_atomically(&destination, json.as_bytes())
        .wrap_err_with(|| format!("could not write {}", destination.display()))?;
    runtime
        .store
        .record_export(investigation.id, &destination.display().to_string())?;
    let count = bundle
        .get("objects")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    println!(
        "Wrote STIX 2.1 bundle with {count} object(s) to {}",
        destination.display()
    );
    Ok(())
}

fn get_object_command(cli: &Cli, query: &str, reference: &str) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let view = crate::query::object_view_for_ref(&runtime.store, investigation.id, reference)
        .map_err(user_err)?;
    print_json(&view)
}

fn objects_command(cli: &Cli, query: &str, kind: Option<&str>) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let workspace = Workspace::new(data);
    let parsed = kind
        .map(crate::query::parse_entity_kind_filter)
        .transpose()
        .map_err(user_err)?;
    let objects = crate::query::list_objects(&workspace, parsed.as_ref());
    print_json(&objects)
}

fn search_around_command(
    cli: &Cli,
    query: &str,
    seed: &str,
    hops: Option<u32>,
    kind: Option<&str>,
    link: Option<&str>,
) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let investigation = runtime.store.require_investigation(query)?;
    let data = runtime.store.load_investigation(investigation.id)?;
    let workspace = Workspace::new(data);
    let kinds = kind
        .map(|value| {
            crate::query::parse_entity_kind_filter(value)
                .map(|kind| vec![kind])
                .map_err(user_err)
        })
        .transpose()?
        .unwrap_or_default();
    let links = link
        .map(|value| vec![value.to_string()])
        .unwrap_or_default();
    let view = crate::query::search_around_view(
        &workspace,
        seed,
        hops.unwrap_or(DEFAULT_SEARCH_HOPS),
        &kinds,
        &links,
    )
    .map_err(user_err)?;
    if cli.json {
        return print_json(&view);
    }
    let theme = cli_theme(cli);
    println!(
        "GRAPH  {} hop(s){}{} nodes{}{} edges",
        view.hops_used,
        theme.glyphs.sep(),
        view.nodes.len(),
        theme.glyphs.sep(),
        view.edges.len()
    );
    if view.nodes.is_empty() {
        println!("  No nodes.");
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(root) = view.nodes.first() {
        print_cli_tree_node(&view, &theme, root.object.id, None, "", &mut seen);
    }
    Ok(())
}

fn cli_entity_tag(kind: &str) -> &'static str {
    crate::tui::Glyphs::entity_tag(&EntityKind::parse(kind))
}

fn print_cli_tree_node(
    view: &crate::query::SearchAroundView,
    theme: &crate::tui::Theme,
    id: uuid::Uuid,
    parent: Option<uuid::Uuid>,
    prefix: &str,
    seen: &mut std::collections::HashSet<uuid::Uuid>,
) {
    let Some(node) = view.nodes.iter().find(|node| node.object.id == id) else {
        return;
    };
    let already = !seen.insert(id);
    let glyph = theme.glyphs.provenance(
        crate::model::Provenance::parse(&node.object.provenance)
            .unwrap_or(crate::model::Provenance::Collected),
    );
    let seen_mark = if already { " (seen)" } else { "" };
    if prefix.is_empty() {
        println!(
            "  {} {} {}{seen_mark}",
            cli_entity_tag(&node.object.kind),
            node.object.value,
            glyph
        );
    }
    if already {
        return;
    }
    let mut children: Vec<_> = view
        .edges
        .iter()
        .filter_map(|edge| {
            let other = if edge.from_id == id {
                edge.to_id
            } else if edge.to_id == id {
                edge.from_id
            } else {
                return None;
            };
            if Some(other) == parent {
                return None;
            }
            Some((other, edge))
        })
        .collect();
    children.sort_by(|left, right| left.1.kind.cmp(&right.1.kind));
    let count = children.len();
    for (index, (child, edge)) in children.into_iter().enumerate() {
        let last = index + 1 == count;
        let branch = if theme.glyphs.ascii {
            if last { "`- " } else { "|- " }
        } else if last {
            "└─ "
        } else {
            "├─ "
        };
        let pad = if last {
            "   "
        } else if theme.glyphs.ascii {
            "|  "
        } else {
            "│  "
        };
        let next = format!("{prefix}{pad}");
        let child_node = view.nodes.iter().find(|node| node.object.id == child);
        let child_seen = seen.contains(&child);
        let label = child_node
            .map(|node| node.object.value.as_str())
            .unwrap_or("?");
        let kind = child_node
            .map(|node| cli_entity_tag(&node.object.kind))
            .unwrap_or("UNK");
        let glyph = child_node
            .map(|node| {
                theme.glyphs.provenance(
                    crate::model::Provenance::parse(&node.object.provenance)
                        .unwrap_or(crate::model::Provenance::Collected),
                )
            })
            .unwrap_or("?");
        let mark = if child_seen { " (seen)" } else { "" };
        println!("{prefix}{branch}{} {kind} {label} {glyph}{mark}", edge.kind);
        if !child_seen {
            print_cli_tree_node(view, theme, child, Some(id), &next, seen);
        }
    }
}

fn resolve_command(cli: &Cli, command: &ResolveCommand) -> Result<()> {
    match command {
        ResolveCommand::Suggest { investigation } => {
            let runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let data = runtime.store.load_investigation(investigation.id)?;
            let workspace = Workspace::new(data);
            print_json(&crate::query::resolution_suggestion_views(&workspace))
        }
        ResolveCommand::Propose {
            investigation,
            entities,
            reason,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            if entities.len() < 2 {
                return Err(user_err("propose needs at least two entity refs"));
            }
            let action = AgentAction::ProposeResolve {
                entities: entities.clone(),
                reason: reason.clone().unwrap_or_default(),
            };
            let proposals = crate::query::propose_actions(
                &mut runtime.store,
                investigation.id,
                "cli",
                vec![action],
            )
            .map_err(user_err)?;
            print_json(&proposals)
        }
        ResolveCommand::Merge {
            investigation,
            entities,
            reason,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let data = runtime.store.load_investigation(investigation.id)?;
            let workspace = Workspace::new(data);
            let mut ids = Vec::new();
            for raw in entities {
                ids.push(resolve_entity_query(&workspace, raw)?);
            }
            if ids.len() < 2 {
                return Err(user_err("merge needs at least two distinct entities"));
            }
            if !confirm_write(cli, "Merge these entities? This writes a resolution bag.")? {
                return Err(user_err("cancelled; nothing written"));
            }
            let canonical = ids[0];
            let actor = runtime.store.identity().user_id.clone();
            let resolution = runtime.store.resolve_entities(
                investigation.id,
                &ids,
                canonical,
                reason.as_deref().unwrap_or("cli merge"),
                &actor,
            )?;
            println!(
                "Merged {} into {}",
                ids.len(),
                workspace
                    .ref_of(resolution.canonical_id)
                    .map(|reference| reference.to_string())
                    .unwrap_or_else(|| canonical.to_string())
            );
            Ok(())
        }
        ResolveCommand::Unresolve {
            investigation,
            target,
        } => {
            let mut runtime = Runtime::load(cli)?;
            let investigation = runtime.store.require_investigation(investigation)?;
            let data = runtime.store.load_investigation(investigation.id)?;
            let workspace = Workspace::new(data);
            let resolution_id = if let Ok(reference) = target.parse::<crate::model::ShortRef>()
                && reference.kind == crate::model::RefKind::Resolution
            {
                workspace
                    .resolve_ref(reference)
                    .ok_or_else(|| user_err(format!("unknown resolution {target}")))?
            } else {
                let entity_id = resolve_entity_query(&workspace, target)?;
                workspace
                    .latest_active_resolution(entity_id)
                    .map(|resolution| resolution.id)
                    .ok_or_else(|| user_err(format!("no active resolution for {target}")))?
            };
            if !confirm_write(cli, "Unresolve this bag? This writes.")? {
                return Err(user_err("cancelled; nothing written"));
            }
            let actor = runtime.store.identity().user_id.clone();
            runtime.store.unresolve_entities(resolution_id, &actor)?;
            println!("Unresolved {target}");
            Ok(())
        }
    }
}

async fn serve_command(
    cli: &Cli,
    bind: Option<&str>,
    token: Option<&str>,
    allow_non_loopback: bool,
) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    let mut options = crate::api::ServeOptions::from_config(&runtime.config.api, bind, token)
        .map_err(user_err)?;
    options.allow_non_loopback = allow_non_loopback;
    crate::api::serve(runtime.store, options).await
}

fn bench_command(cli: &Cli, sizes: Option<&str>) -> Result<()> {
    let mut runtime = Runtime::load(cli)?;
    let counts = parse_bench_sizes(sizes)?;
    let mut rows = Vec::new();
    for count in counts {
        rows.push(run_bench_size(&mut runtime.store, count)?);
    }
    print_json(&rows)
}

fn parse_bench_sizes(sizes: Option<&str>) -> Result<Vec<usize>> {
    let raw = sizes.unwrap_or("1000,10000,100000");
    let mut counts = Vec::new();
    for part in raw.split(',') {
        let count: usize = part
            .trim()
            .parse()
            .map_err(|_| user_err(format!("invalid bench size {part:?}")))?;
        if count == 0 {
            return Err(user_err("bench size must be > 0"));
        }
        counts.push(count);
    }
    Ok(counts)
}

fn run_bench_size(store: &mut Store, entity_count: usize) -> Result<serde_json::Value> {
    let name = format!("bench-{entity_count}");
    if store.find_investigation(&name)?.is_some() {
        // Unique name per run so repeated benches do not collide.
    }
    let unique = format!("{name}-{}", Uuid::new_v4().simple());
    let investigation = store.create_investigation(&unique, "scale fixture")?;
    let seed_started = std::time::Instant::now();
    store.seed_scale_graph(investigation.id, entity_count)?;
    let seed_ms = seed_started.elapsed().as_secs_f64() * 1000.0;
    let load_started = std::time::Instant::now();
    let data = store.load_investigation(investigation.id)?;
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let index_started = std::time::Instant::now();
    let mut workspace = Workspace::new(data);
    let index_ms = index_started.elapsed().as_secs_f64() * 1000.0;
    let seed_id = workspace.data.entities.first().map(|entity| entity.id);
    let search_ms = if let Some(seed_id) = seed_id {
        let started = std::time::Instant::now();
        let query = crate::model::SearchAroundQuery::new(vec![seed_id], 2);
        let _ = crate::model::search_around(&workspace.data, &query);
        started.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };
    let patch_started = std::time::Instant::now();
    let finding = store.create_finding(crate::storage::NewFinding {
        investigation_id: investigation.id,
        page_id: None,
        title: "bench finding".into(),
        details: String::new(),
        confidence: crate::model::Confidence::Low,
        provenance: crate::model::Provenance::Analyst,
        source_ids: Vec::new(),
    })?;
    workspace.apply_delta(crate::model::WorkspaceDelta {
        findings: vec![finding],
        ..crate::model::WorkspaceDelta::default()
    });
    let patch_ms = patch_started.elapsed().as_secs_f64() * 1000.0;
    let reload_started = std::time::Instant::now();
    let _ = store.load_investigation(investigation.id)?;
    let reload_ms = reload_started.elapsed().as_secs_f64() * 1000.0;
    Ok(serde_json::json!({
        "entities": entity_count,
        "seed_ms": seed_ms,
        "load_ms": load_ms,
        "index_ms": index_ms,
        "search_around_2hop_ms": search_ms,
        "incremental_finding_patch_ms": patch_ms,
        "full_reload_ms": reload_ms,
        "ui_mutation_budget_ms": 50,
    }))
}

fn resolve_entity_query(workspace: &Workspace, query: &str) -> Result<uuid::Uuid> {
    if let Ok(uuid) = uuid::Uuid::parse_str(query) {
        if workspace.entity(uuid).is_some() {
            return Ok(uuid);
        }
        return Err(user_err(format!("unknown entity {query}")));
    }
    if let Ok(reference) = query.parse::<crate::model::ShortRef>()
        && let Some(id) = workspace.resolve_ref(reference)
    {
        return Ok(id);
    }
    Err(user_err(format!(
        "unknown entity {query:?}; use a UUID or short ref such as E-1"
    )))
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
    let failed = checks
        .iter()
        .any(|check| check.status == crate::doctor::CheckStatus::Warn);
    if cli.json {
        print_json(&checks)?;
        if failed {
            return Err(user_err("doctor reported warnings"));
        }
        return Ok(());
    }
    for check in checks {
        println!(
            "{:<16} {:<8} {}",
            check.name,
            check.status.symbol(),
            check.detail
        );
    }
    if failed {
        return Err(user_err("doctor reported warnings"));
    }
    Ok(())
}

fn format_glance_buckets(
    counts: &crate::app::case::GlanceCounts,
    theme: &crate::tui::Theme,
) -> String {
    let parts: Vec<String> = counts
        .nonzero_buckets()
        .into_iter()
        .map(|(provenance, count)| format!("{} {count}", theme.glyphs.provenance(provenance)))
        .collect();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join("  ")
    }
}

fn show_config(cli: &Cli) -> Result<()> {
    let runtime = Runtime::load(cli)?;
    if cli.json {
        return print_json(&serde_json::json!({
            "config": runtime.paths.config_file.display().to_string(),
            "data": runtime.paths.data_dir.display().to_string(),
            "database": runtime.paths.database().display().to_string(),
            "logs": runtime.paths.log_dir().display().to_string(),
        }));
    }
    println!("config    {}", runtime.paths.config_file.display());
    println!("data      {}", runtime.paths.data_dir.display());
    println!("database  {}", runtime.paths.database().display());
    println!("logs      {}", runtime.paths.log_dir().display());
    Ok(())
}

fn parse_watch_interval(seconds: u64) -> Result<u64> {
    if seconds > crate::config::MAX_WATCH_INTERVAL_SECONDS {
        return Err(user_err(format!(
            "watch interval {seconds} seconds is too large; maximum is {} seconds",
            crate::config::MAX_WATCH_INTERVAL_SECONDS
        )));
    }
    Ok(seconds.max(1))
}

fn parse_cooccur_window(hours: u64) -> Result<chrono::Duration> {
    if hours > crate::model::MAX_WINDOW_HOURS {
        return Err(user_err(format!(
            "co-occurrence window {hours} hours is too large; maximum is {} hours",
            crate::model::MAX_WINDOW_HOURS
        )));
    }
    chrono::Duration::try_hours(hours as i64).ok_or_else(|| {
        user_err(format!(
            "co-occurrence window {hours} hours is too large; maximum is {} hours",
            crate::model::MAX_WINDOW_HOURS
        ))
    })
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

fn truncate(text: &str, width: usize, ascii: bool) -> String {
    crate::tui::widgets::ellipsize(text, width, ascii)
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
        let stix = Cli::try_parse_from(["os-int", "export", "Case", "--format", "stix"])
            .expect("stix export");
        match stix.command {
            Some(Command::Export { format, .. }) => assert_eq!(format, "stix"),
            other => panic!("expected export {other:?}"),
        }
        let get = Cli::try_parse_from(["os-int", "get", "Case", "E-1"]).expect("get");
        assert!(matches!(get.command, Some(Command::Get { .. })));
        let around = Cli::try_parse_from(["os-int", "search-around", "Case", "E-1", "--hops", "2"])
            .expect("search-around");
        assert!(matches!(
            around.command,
            Some(Command::SearchAround { hops: Some(2), .. })
        ));
        let serve =
            Cli::try_parse_from(["os-int", "serve", "--bind", "127.0.0.1:0"]).expect("serve");
        match serve.command {
            Some(Command::Serve {
                allow_non_loopback, ..
            }) => assert!(!allow_non_loopback),
            other => panic!("expected serve {other:?}"),
        }
        let plain = Cli::try_parse_from(["os-int", "--plain"]).expect("plain");
        assert!(plain.plain);
        let review = Cli::try_parse_from(["os-int", "--yes", "review", "accept", "Case", "X-1"])
            .expect("review accept");
        assert!(review.yes);
        match review.command {
            Some(Command::Review {
                command: ReviewCommand::Accept { reference, .. },
            }) => assert_eq!(reference, "X-1"),
            other => panic!("expected review accept {other:?}"),
        }
        let case = Cli::try_parse_from(["os-int", "--json", "case", "Case"]).expect("case");
        assert!(case.json);
        assert!(matches!(case.command, Some(Command::Case { .. })));
        let preview =
            Cli::try_parse_from(["os-int", "theme", "preview", "--all"]).expect("preview");
        match preview.command {
            Some(Command::Theme {
                command: ThemeCommand::Preview { all, name },
            }) => {
                assert!(all);
                assert!(name.is_none());
            }
            other => panic!("expected theme preview {other:?}"),
        }
        let named =
            Cli::try_parse_from(["os-int", "theme", "preview", "phosphor"]).expect("named theme");
        match named.command {
            Some(Command::Theme {
                command: ThemeCommand::Preview { name, all },
            }) => {
                assert_eq!(name.as_deref(), Some("phosphor"));
                assert!(!all);
            }
            other => panic!("expected theme preview {other:?}"),
        }
        for name in ["unknown", "default", "high-contrast"] {
            let cli = Cli::try_parse_from(["os-int", "theme", "preview", name]).expect("parse");
            let error = match &cli.command {
                Some(Command::Theme { command }) => theme_command(&cli, command).expect_err(name),
                other => panic!("expected theme {other:?}"),
            };
            let text = error.to_string();
            assert!(text.contains("unknown theme"), "{text}");
            assert!(text.contains("dark"), "{text}");
            assert!(text.contains("phosphor"), "{text}");
        }
        let exposed = Cli::try_parse_from([
            "os-int",
            "serve",
            "--bind",
            "0.0.0.0:8743",
            "--allow-non-loopback",
        ])
        .expect("allow");
        match exposed.command {
            Some(Command::Serve {
                allow_non_loopback, ..
            }) => assert!(allow_non_loopback),
            other => panic!("expected serve {other:?}"),
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

    #[test]
    fn huge_watch_interval_is_a_user_error() {
        let error = parse_watch_interval(u64::MAX).expect_err("too large");
        let text = error.to_string();
        assert!(
            text.contains("too large") || text.contains("interval"),
            "{text}"
        );
        assert!(!text.contains("panic"), "{text}");
        assert_eq!(parse_watch_interval(30).expect("ok"), 30);
        assert_eq!(parse_watch_interval(0).expect("zero becomes 1"), 1);
    }

    #[test]
    fn confirm_write_refuses_without_yes_when_stdin_is_not_a_tty() {
        let cli = Cli::try_parse_from(["os-int", "review", "accept", "Case", "X-1"]).expect("cli");
        assert!(!cli.yes);
        let error = confirm_write(&cli, "Accept AI item?").expect_err("non-tty");
        let text = error.to_string();
        assert!(text.contains("--yes"), "{text}");
        assert!(
            text.contains("not a TTY") || text.contains("Refusing"),
            "{text}"
        );
    }

    #[test]
    fn cli_review_accept_is_blocked_when_a_citation_is_missing() {
        let item = crate::app::review::ReviewItem {
            kind: crate::app::review::ReviewKind::Agent,
            proposal_id: Some(Uuid::new_v4()),
            run_id: None,
            merge: None,
            short_ref: Some("X-1".into()),
            title: "blocked".into(),
            group: "X-1 Q-1 · agent".into(),
            preview: Vec::new(),
            citations: vec![crate::app::review::ReviewCitation {
                reference: "S-99".into(),
                present: false,
                label: "missing".into(),
            }],
            rationale: String::new(),
            written_as: Some(crate::model::Provenance::Agent),
        };
        let error = review_write_guard(&item, true).expect_err("blocked");
        assert!(error.to_string().contains("missing source"), "{error}");
        assert!(item.matches_reference("X-1"));
        assert!(!item.matches_reference("X-10"));
    }

    #[test]
    fn cli_accept_copy_matches_item_provenance() {
        let ai = crate::app::review::ReviewItem {
            kind: crate::app::review::ReviewKind::Agent,
            proposal_id: Some(Uuid::new_v4()),
            run_id: None,
            merge: None,
            short_ref: Some("X-1".into()),
            title: "phish".into(),
            group: "X-1".into(),
            preview: Vec::new(),
            citations: Vec::new(),
            rationale: String::new(),
            written_as: Some(crate::model::Provenance::Agent),
        };
        assert!(review_confirm_prompt(&ai, true).contains("agent provenance"));
        assert!(review_accepted_copy(ai.written_as, true).contains('*'));
        assert!(review_accepted_copy(ai.written_as, false).contains('◆'));
        let analyst = crate::app::review::ReviewItem {
            written_as: Some(crate::model::Provenance::Analyst),
            ..ai
        };
        assert!(review_confirm_prompt(&analyst, true).contains("analyst provenance"));
        assert!(review_accepted_copy(analyst.written_as, false).contains("Analyst"));
    }

    #[test]
    fn json_flag_is_accepted_on_playbook_team_marking_cooccur_and_doctor() {
        for args in [
            ["os-int", "--json", "playbook", "list"].as_slice(),
            ["os-int", "--json", "team", "list", "Case"].as_slice(),
            ["os-int", "--json", "marking", "get", "Case"].as_slice(),
            ["os-int", "--json", "cooccur", "Case"].as_slice(),
            ["os-int", "--json", "doctor"].as_slice(),
        ] {
            let cli = Cli::try_parse_from(args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
            assert!(cli.json, "{args:?}");
        }
    }

    #[test]
    fn glance_buckets_omit_zero_counts_and_skip_hollow_diamond_for_agent_findings() {
        let mut config = Config::default();
        config.ui.ascii = true;
        let env = crate::tui::TerminalEnv {
            no_color: true,
            ..crate::tui::TerminalEnv::default()
        };
        let theme = crate::tui::Theme::resolve(&config, &env);
        let counts = crate::app::case::GlanceCounts {
            total: 1,
            agent: 1,
            ..crate::app::case::GlanceCounts::default()
        };
        let text = format_glance_buckets(&counts, &theme);
        assert!(text.contains('*'), "{text}");
        assert!(!text.contains('◇'), "{text}");
        assert!(!text.contains('X'), "{text}");
        let cli = Cli::try_parse_from(["os-int", "--plain", "case", "Case"]).expect("plain");
        assert!(cli.plain);
        let theme = cli_theme(&cli);
        assert!(theme.glyphs.ascii);
        assert_eq!(theme.glyphs.external, "X");
        assert_eq!(theme.glyphs.ai, "*");
    }

    #[test]
    fn cooccur_window_rejects_values_that_would_panic() {
        let error = parse_cooccur_window(2_562_047_788_016).expect_err("must reject");
        assert!(error.to_string().contains("too large"), "{error}");
        let error = parse_cooccur_window(u64::MAX).expect_err("must reject");
        assert!(error.to_string().contains("too large"), "{error}");
        let ok = parse_cooccur_window(24).expect("default-sized window");
        assert_eq!(ok, chrono::Duration::hours(24));
    }

    #[test]
    fn json_flag_is_accepted_on_config() {
        let cli = Cli::try_parse_from(["os-int", "--json", "config"]).expect("config json");
        assert!(cli.json);
        assert!(matches!(cli.command, Some(Command::Config)));
    }

    #[test]
    fn jobs_plain_marks_use_ascii_status_symbols() {
        assert_eq!(JobStatus::Queued.symbol(true), "...");
        assert_eq!(JobStatus::Running.symbol(true), "*");
        assert_eq!(JobStatus::Succeeded.symbol(true), "ok");
        for status in [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Succeeded,
            JobStatus::Failed,
            JobStatus::Cancelled,
        ] {
            let mark = status.symbol(true);
            assert!(
                mark.bytes().all(|byte| byte.is_ascii()),
                "{status:?} {mark}"
            );
            assert!(
                !mark.contains('…') && !mark.contains('●') && !mark.contains('✓'),
                "{mark}"
            );
        }
    }
}
