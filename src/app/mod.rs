//! Application state and action handling. Testable without a terminal.
//!
//! [`App`] owns the config, the database, the loaded investigation, and the job manager.
//! [`Action`] values come from the keymap or from a finished job. `App::dispatch` is the only
//! place that changes that state. The TUI draws whatever `App` currently holds.
//!
//! Per-investigation UI state (last view, page, and agent thread) is stored in `ui_state` and
//! restored when the case is opened again.

pub mod action;
pub mod case;
pub mod commands;
pub mod filter;
pub mod graph;
pub mod input;
pub mod resolution;
pub mod review;
pub mod timeline;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use uuid::Uuid;

pub use action::Action;
pub use graph::GraphState;
pub use input::Input;
pub use resolution::PendingResolve;
pub use timeline::TimelineState;

use crate::agent::{AgentContext, AgentRequest, AgentStatus, ContextFocus, run_agent};
use crate::collectors::tool::run_tool;
use crate::collectors::{
    CollectionOutput, CollectorContext, CollectorRegistry, CollectorSettings, NetContext,
};
use crate::config::Config;
use crate::files::InvestigationFiles;
use crate::jobs::{JobEvent, JobManager, JobOutcome, JobOutput};
use crate::model::{
    AgentAction, AgentRun, Artifact, ArtifactKind, Confidence, JobRecord, JobStatus, NewArtifact,
    Page, PageEntityLink, PageKind, Proposal, ProposalStatus, Provenance, Target, Workspace,
    WorkspaceDelta, now,
};
use crate::paths::AppPaths;
use crate::report::write_report;
use crate::storage::{ApplyContext, GLOBAL_SCOPE, NewFinding, NewPage, Store};
use crate::tools::{Activity, ToolEntry, ToolRegistry};

use self::commands::filtered;

/// Rail destinations are Case, Entities, Evidence, Graph, Review, and Jobs (`1`–`6`).
/// Cut screens stay reachable from the palette or Inspector tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// First-run and no-investigation screen: new, open, doctor, help.
    Welcome,
    /// Case home: targets, at-a-glance counts, and next actions. Rail `1`.
    Overview,
    /// Combined findings and sources. Rail `3`.
    Evidence,
    /// AI, playbook, and merge proposals. Rail `5`.
    Review,
    /// Subjects in this investigation. Palette `:pages`; Case shows them as targets.
    Pages,
    /// Observations. Inspector Sources tab and Evidence; palette `:sources`.
    Sources,
    /// Claims. Shown on Evidence.
    Findings,
    /// Normalized identifiers and the relationships between them. Rail `2`.
    Entities,
    /// Time lens on Evidence / Graph (`t`). Palette `:timeline`.
    Timeline,
    /// Notes live on the Inspector tab. Palette `:notes` and `e`.
    Notes,
    /// Agent transcript. Inspector for a Review run; palette `:agent`.
    Agent,
    /// Background collectors, tools, agent calls, and reports. Rail `6`.
    Jobs,
    /// External programs. Palette `:tools`. Active tools are refused.
    Tools,
    /// Active agent profile and theme. Palette `:settings`.
    Settings,
    /// Files stored for this investigation. Palette `:artifacts`.
    Artifacts,
    /// Typed properties, catalogued links, sources, and findings for one object.
    Object,
    /// Link-analysis canvas: entities as nodes, relationships as edges, Search Around.
    Graph,
    /// Append-only governance log. Inspector History tab and palette `:audit`.
    Audit,
}

impl View {
    pub fn as_str(self) -> &'static str {
        match self {
            View::Welcome => "welcome",
            View::Overview => "case",
            View::Evidence => "evidence",
            View::Review => "review",
            View::Pages => "pages",
            View::Sources => "sources",
            View::Findings => "findings",
            View::Entities => "entities",
            View::Timeline => "timeline",
            View::Notes => "notes",
            View::Agent => "agent",
            View::Jobs => "jobs",
            View::Tools => "tools",
            View::Settings => "settings",
            View::Artifacts => "artifacts",
            View::Object => "object",
            View::Graph => "graph",
            View::Audit => "audit",
        }
    }

    pub fn parse(value: &str) -> Option<View> {
        Some(match value {
            "welcome" => View::Welcome,
            "overview" | "case" => View::Overview,
            "evidence" => View::Evidence,
            "review" => View::Review,
            "pages" => View::Pages,
            "sources" => View::Sources,
            "findings" => View::Findings,
            "entities" => View::Entities,
            "timeline" => View::Timeline,
            "notes" => View::Notes,
            "agent" => View::Agent,
            "jobs" => View::Jobs,
            "tools" => View::Tools,
            "settings" => View::Settings,
            "artifacts" => View::Artifacts,
            "object" => View::Object,
            "graph" => View::Graph,
            "audit" => View::Audit,
            _ => return None,
        })
    }

    /// Label shown on the Rail and in the header.
    pub fn title(self) -> &'static str {
        match self {
            View::Welcome => "Welcome",
            View::Overview => "Case",
            View::Evidence => "Evidence",
            View::Review => "Review",
            View::Pages => "Targets",
            View::Sources => "Sources",
            View::Findings => "Findings",
            View::Entities => "Entities",
            View::Timeline => "Timeline",
            View::Notes => "Notes",
            View::Agent => "Agent",
            View::Jobs => "Jobs",
            View::Tools => "Tools",
            View::Settings => "Settings",
            View::Artifacts => "Artifacts",
            View::Object => "Object",
            View::Graph => "Graph",
            View::Audit => "Audit",
        }
    }

    pub fn is_rail(self) -> bool {
        matches!(
            self,
            View::Overview
                | View::Entities
                | View::Evidence
                | View::Graph
                | View::Review
                | View::Jobs
        )
    }
}

/// Which column has the keyboard focus. `Tab` / `h` `l` cycle Rail → Board → Inspector.
/// The Inspector is never hidden: below 110 columns it becomes a bottom drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The six-item Rail (Case, Entities, Evidence, Graph, Review, Jobs).
    Nav,
    /// The list or body for [`View`].
    Main,
    /// The Inspector for the selected record.
    Detail,
}

/// Inspector section. P2 shows the tabs; P3 adds source footnotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorTab {
    Props,
    Sources,
    Notes,
    History,
}

impl InspectorTab {
    pub const ALL: [InspectorTab; 4] = [
        InspectorTab::Props,
        InspectorTab::Sources,
        InspectorTab::Notes,
        InspectorTab::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            InspectorTab::Props => "Props",
            InspectorTab::Sources => "Sources",
            InspectorTab::Notes => "Notes",
            InspectorTab::History => "History",
        }
    }
}

/// A modal drawn over the workbench. Keys are handled by the overlay until it closes.
/// Letters insert text while a form, search box, palette, or agent prompt is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The workbench itself is receiving keys.
    None,
    /// Keyboard cheat sheet. `PgUp` / `PgDn` scroll it.
    Help,
    /// Filterable list of commands from `app::commands::COMMANDS`.
    Palette { query: Input, selected: usize },
    /// Search hits across the open investigation.
    Search { query: Input, selected: usize },
    /// The question that will be sent to the active agent.
    AgentPrompt { input: Input },
    /// A multi-field form (new investigation, new page, new finding, settings).
    Form(FormState),
    /// A yes/no prompt that runs `command` when confirmed.
    Confirm {
        title: String,
        message: String,
        command: String,
    },
    /// Scrollable text, used for long records that are not a form.
    Text {
        title: String,
        body: String,
        scroll: u16,
    },
    /// The same checks `os-int doctor` prints.
    Doctor,
    /// Inline `/` filter for the current table. Typing updates the board live.
    TableFilter { query: Input },
    /// Palette `:theme` live cycle. y saves, Esc reverts. Enter does not write.
    ThemeCycle { original: String, index: usize },
}

/// Which form [`Overlay::Form`] is editing. The field list is built to match the kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    /// Name and description of a new case.
    NewInvestigation,
    /// Name, UUID, or prefix of a case to open.
    OpenInvestigation,
    /// Page type and target. Left and right cycle the type.
    NewPage,
    /// Title, details, confidence, and cited source ids.
    NewFinding,
    /// Body of a note.
    NewNote,
    /// Active agent profile, optional program override, timeout, and theme.
    SettingsAgent,
    /// Ingest a STIX, MISP, CSV, JSON, or Evidence Batch file.
    Ingest,
    /// Run a bundled playbook or YAML file.
    Playbook,
    /// Seed the agent prompt from a playbook.
    PlaybookPrompt,
    /// Watch set: re-run collect steps; dry-run lists planned Actions.
    Watch,
    /// Set the investigation TLP marking.
    Marking,
    /// Add a case member.
    TeamMember,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormState {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: Input,
    pub hint: String,
}

impl FormState {
    fn new(kind: FormKind, fields: Vec<Field>) -> Self {
        Self {
            kind,
            fields,
            index: 0,
        }
    }

    fn current_mut(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.index)
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub label: String,
    pub kind: String,
    pub id: Uuid,
    pub view: View,
}

/// One row on the Evidence board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceRow {
    Finding(Uuid),
    Source(Uuid),
}

pub struct App {
    pub config: Config,
    pub paths: AppPaths,
    pub store: Store,
    pub tools: Arc<ToolRegistry>,
    pub collectors: CollectorRegistry,
    pub net: Result<Arc<NetContext>, String>,
    pub jobs: JobManager,
    pub job_records: HashMap<Uuid, JobRecord>,
    pub workspace: Option<Workspace>,
    pub view: View,
    pub pane: Pane,
    pub overlay: Overlay,
    pub selected: usize,
    pub scroll: u16,
    pub nav_index: usize,
    pub page_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub source_id: Option<Uuid>,
    pub finding_id: Option<Uuid>,
    pub agent_run_id: Option<Uuid>,
    pub agent_thread: Option<Uuid>,
    pub object_return_view: Option<View>,
    pub graph: GraphState,
    pub timeline: TimelineState,
    pub merge_marks: Vec<Uuid>,
    pub pending_resolve: Option<PendingResolve>,
    pub pending_unresolve: Option<Uuid>,
    pub pending_merges: Vec<PendingResolve>,
    /// Scored merge suggestions the analyst rejected this session.
    pub dismissed_suggestions: Vec<Vec<Uuid>>,
    pub review_armed: bool,
    pub review_marks: Vec<usize>,
    pub board_zoomed: bool,
    /// Last layout was the narrow Inspector drawer. Resize animates only on a false→true edge.
    drawer_narrow: bool,
    pub time_lens: bool,
    pub inspector_tab: InspectorTab,
    pub board_filter: String,
    pub status: String,
    pub dirty: bool,
    pub should_quit: bool,
    pub agent: AgentStatus,
    pub streaming: HashMap<Uuid, String>,
    pub last_error: Option<String>,
    pub pending_editor: Option<EditorSession>,
    /// Last board row id used to remap selection across filter/sort edits.
    board_anchor_id: Option<Uuid>,
    /// Bumped on workspace content changes so BoardListCache cannot outlive the case.
    workspace_rev: u64,
    board_list_cache: RefCell<Option<BoardListCache>>,
    /// Motion engine. Idle holds no timer. Ticks never write.
    pub motion: crate::tui::Animator,
    /// `--plain` / screen-reader: linear layout, ASCII, motion off.
    pub plain: bool,
}

struct BoardListCache {
    key: String,
    entities: Option<Vec<Uuid>>,
    evidence: Option<Vec<EvidenceRow>>,
    jobs: Option<Vec<Uuid>>,
    tools: Option<Vec<usize>>,
}

/// External `$EDITOR` session for long-form page or entity notes.
#[derive(Clone, Debug)]
pub struct EditorSession {
    pub path: PathBuf,
    pub page_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
}

impl App {
    pub fn new(config: Config, paths: AppPaths, store: Store) -> Self {
        let tools = Arc::new(ToolRegistry::detect(&config.tools));
        let agent = AgentStatus::resolve(&config, &tools);
        let net = NetContext::new(&config.network).map(Arc::new);
        let max_jobs = config.collectors.max_concurrent_jobs.max(1);
        let collectors = CollectorRegistry::from_config(&config);
        let motion = crate::tui::Animator::from_config(
            &config,
            &crate::tui::TerminalEnv::from_process(),
            false,
        );
        let app = Self {
            config,
            paths,
            store,
            tools,
            collectors,
            net,
            jobs: JobManager::new(max_jobs),
            job_records: HashMap::new(),
            workspace: None,
            view: View::Welcome,
            pane: Pane::Nav,
            overlay: Overlay::None,
            selected: 0,
            scroll: 0,
            nav_index: 0,
            page_id: None,
            entity_id: None,
            source_id: None,
            finding_id: None,
            agent_run_id: None,
            agent_thread: None,
            object_return_view: None,
            graph: GraphState::default(),
            timeline: TimelineState::default(),
            merge_marks: Vec::new(),
            pending_resolve: None,
            pending_unresolve: None,
            pending_merges: Vec::new(),
            dismissed_suggestions: Vec::new(),
            review_armed: false,
            review_marks: Vec::new(),
            board_zoomed: false,
            drawer_narrow: false,
            time_lens: false,
            inspector_tab: InspectorTab::Props,
            board_filter: String::new(),
            status: "Press [?] for help, Ctrl+K for commands.".into(),
            dirty: true,
            should_quit: false,
            agent,
            streaming: HashMap::new(),
            last_error: None,
            pending_editor: None,
            board_anchor_id: None,
            workspace_rev: 0,
            board_list_cache: RefCell::new(None),
            motion,
            plain: false,
        };
        let _ = app.store.mark_interrupted_jobs();
        app
    }

    pub fn apply_plain_mode(&mut self) {
        self.plain = true;
        self.config.ui.ascii = true;
        self.config.ui.color = "none".into();
        self.motion = crate::tui::Animator::from_config(
            &self.config,
            &crate::tui::TerminalEnv::from_process(),
            true,
        );
    }

    pub fn is_plain_linear(&self) -> bool {
        self.plain
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn agent_job_running(&self) -> bool {
        self.job_records
            .values()
            .any(|job| job.status.is_active() && job.kind == "agent")
    }

    pub fn sync_motion(&mut self) {
        let mut collector = false;
        let mut agent = false;
        let mut observing = false;
        let jobs: Vec<(uuid::Uuid, String, bool, Option<String>)> = self
            .job_records
            .values()
            .filter(|job| job.status.is_active())
            .map(|job| {
                (
                    job.id,
                    job.kind.clone(),
                    job.kind == "agent",
                    job.progress.clone(),
                )
            })
            .collect();
        for (id, kind, is_agent, progress) in jobs {
            if is_agent {
                agent = true;
                continue;
            }
            collector = true;
            if kind.starts_with("collector:") {
                observing = true;
            }
            self.motion.ensure_spinner(format!("spin:{id}"));
            if progress
                .as_deref()
                .and_then(crate::tui::motion::parse_known_progress)
                .is_some()
            {
                self.motion.sync_progress(&format!("prog:{id}"), true);
            }
        }
        self.motion.sync_agent(agent);
        self.motion.sync_radar(observing);
        if !collector {
            self.motion.stop_prefix("spin:");
            self.motion
                .stop_kind(crate::tui::motion::AnimKind::JobSpinner);
            self.motion.stop_prefix("prog:");
        }
        if !collector && !agent {
            self.motion.clear_reduced_busy();
        }
    }

    pub fn clear_graph_plus(&mut self) {
        self.graph.plus_ids.clear();
    }

    const THEME_CYCLE: [&'static str; 5] = ["dark", "light", "mono", "phosphor", "amber"];

    fn open_theme_cycle(&mut self) {
        let original = self.config.ui.theme.clone();
        let current = crate::tui::ThemeName::parse(&original)
            .map(|name| name.as_str())
            .unwrap_or("dark");
        let index = Self::THEME_CYCLE
            .iter()
            .position(|name| *name == current)
            .unwrap_or(0);
        self.overlay = Overlay::ThemeCycle { original, index };
        self.apply_theme_preview(index);
    }

    fn apply_theme_preview(&mut self, index: usize) {
        let name = Self::THEME_CYCLE[index % Self::THEME_CYCLE.len()];
        self.config.ui.theme = name.into();
        self.status = format!("Theme {name} — y saves, Esc reverts.");
        self.mark_dirty();
    }

    fn save_theme_cycle(&mut self) -> Result<()> {
        if let Overlay::ThemeCycle { index, .. } = self.overlay {
            let name = Self::THEME_CYCLE[index % Self::THEME_CYCLE.len()];
            self.config.ui.theme = name.into();
            self.config.save(&self.paths.config_file)?;
            self.status = format!("Saved theme {name}.");
        }
        self.overlay = Overlay::None;
        Ok(())
    }

    fn revert_theme_cycle(&mut self) {
        if let Overlay::ThemeCycle { original, .. } = &self.overlay {
            self.config.ui.theme = original.clone();
        }
        self.overlay = Overlay::None;
        self.status = "Theme unchanged.".into();
        self.mark_dirty();
    }

    pub fn investigations(&self) -> Vec<crate::model::InvestigationSummary> {
        self.store.list_investigations().unwrap_or_default()
    }

    pub fn current_role_label(&self) -> String {
        let Some(workspace) = self.workspace.as_ref() else {
            return "—".into();
        };
        self.store
            .role_for(workspace.id())
            .map(|role| role.as_str().to_string())
            .unwrap_or_else(|_| "—".into())
    }

    pub fn nav_items(&self) -> Vec<(&'static str, View)> {
        vec![
            ("Case", View::Overview),
            ("Entities", View::Entities),
            ("Evidence", View::Evidence),
            ("Graph", View::Graph),
            ("Review", View::Review),
            ("Jobs", View::Jobs),
        ]
    }

    pub fn open_last_or_welcome(&mut self) -> Result<()> {
        let list = self.investigations();
        if list.is_empty() {
            self.view = View::Welcome;
            return Ok(());
        }
        if let Some(id) = self.store.get_state(GLOBAL_SCOPE, "last_investigation")?
            && let Ok(uuid) = Uuid::parse_str(&id)
            && self.store.get_investigation(uuid)?.is_some()
        {
            return self.open_investigation(uuid);
        }
        self.open_investigation(list[0].investigation.id)
    }

    pub fn open_investigation(&mut self, id: Uuid) -> Result<()> {
        let data = self.store.load_investigation(id)?;
        self.workspace = Some(Workspace::new(data));
        self.touch_workspace();
        self.view = View::Overview;
        self.page_id = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.data.pages.first().map(|page| page.id));
        self.entity_id = self.workspace.as_ref().and_then(|workspace| {
            workspace.data.pages.first().and_then(|page| {
                page.entity_id
                    .or_else(|| workspace.page_entity_ids(page.id).into_iter().next())
            })
        });
        self.selected = 0;
        self.nav_index = 0;
        self.overlay = Overlay::None;
        self.restore_session(id)?;
        self.status = format!(
            "Opened {}",
            self.workspace
                .as_ref()
                .map(|workspace| workspace.investigation().name.as_str())
                .unwrap_or("investigation")
        );
        self.store
            .set_state(GLOBAL_SCOPE, "last_investigation", &id.to_string())?;
        self.mark_dirty();
        Ok(())
    }

    fn restore_session(&mut self, investigation_id: Uuid) -> Result<()> {
        let scope = investigation_id.to_string();
        if let Some(page) = self.store.get_state(&scope, "page")?
            && let Ok(page_id) = Uuid::parse_str(&page)
            && self
                .workspace
                .as_ref()
                .is_some_and(|workspace| workspace.page(page_id).is_some())
        {
            self.page_id = Some(page_id);
        }
        if let Some(entity) = self.store.get_state(&scope, "entity")? {
            if entity.is_empty() {
                self.entity_id = None;
            } else if let Ok(entity_id) = Uuid::parse_str(&entity)
                && self
                    .workspace
                    .as_ref()
                    .is_some_and(|workspace| workspace.entity(entity_id).is_some())
            {
                self.entity_id = Some(entity_id);
            }
        }
        if let Some(id) = self.entity_id {
            self.entity_id = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(id).map(|_| workspace.canonical_id(id)));
        }
        if let Some(hops) = self.store.get_state(&scope, "graph_hops")?
            && let Ok(hops) = hops.parse::<u32>()
        {
            self.graph.hops = hops.min(crate::model::MAX_SEARCH_HOPS);
        }
        if let Some(seeds) = self.store.get_state(&scope, "graph_seeds")? {
            self.graph.seed_ids = seeds
                .split(',')
                .filter_map(|part| Uuid::parse_str(part.trim()).ok())
                .filter(|id| {
                    self.workspace
                        .as_ref()
                        .is_some_and(|workspace| workspace.entity(*id).is_some())
                })
                .collect();
        }
        if let Some(stored) = self.store.get_state(&scope, "view")?
            && let Some(parsed) = View::parse(&stored)
            && parsed != View::Welcome
        {
            self.time_lens = parsed == View::Timeline;
            let view = Self::canonicalize_view(parsed);
            self.view = view;
            if view == View::Object {
                self.select_nav_for_object_view();
            } else if view == View::Graph {
                self.rebuild_graph_after_restore();
            } else if let Some(index) = self
                .nav_items()
                .iter()
                .position(|(_, candidate)| *candidate == view)
            {
                self.nav_index = index;
            }
        }
        if let Some(thread) = self.store.get_state(&scope, "agent_thread")? {
            self.agent_thread = Uuid::parse_str(&thread).ok();
        } else if let Some(run) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.data.agent_runs.last())
        {
            self.agent_thread = Some(run.thread_id);
            self.agent_run_id = Some(run.id);
        }
        if let Some(run_id) = self.agent_thread.and_then(|thread| {
            self.workspace.as_ref().and_then(|workspace| {
                workspace
                    .data
                    .agent_runs
                    .iter()
                    .rev()
                    .find(|run| run.thread_id == thread)
                    .map(|run| run.id)
            })
        }) {
            self.agent_run_id = Some(run_id);
        }
        if let Some(stored) = self.store.get_state(&scope, "dismissed_suggestions")? {
            self.dismissed_suggestions = stored
                .split(';')
                .filter(|group| !group.is_empty())
                .map(|group| {
                    group
                        .split(',')
                        .filter_map(|part| Uuid::parse_str(part.trim()).ok())
                        .collect::<Vec<_>>()
                })
                .filter(|ids| !ids.is_empty())
                .collect();
        }
        Ok(())
    }

    fn save_session(&self) -> Result<()> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Ok(());
        };
        let scope = workspace.id().to_string();
        self.store.set_state(&scope, "view", self.view.as_str())?;
        if let Some(page_id) = self.page_id {
            self.store.set_state(&scope, "page", &page_id.to_string())?;
        }
        self.store.set_state(
            &scope,
            "entity",
            &self
                .entity_id
                .map(|entity_id| entity_id.to_string())
                .unwrap_or_default(),
        )?;
        self.store
            .set_state(&scope, "graph_hops", &self.graph.hops.to_string())?;
        let seeds = self
            .graph
            .seed_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.store.set_state(&scope, "graph_seeds", &seeds)?;
        if let Some(thread) = self.agent_thread {
            self.store
                .set_state(&scope, "agent_thread", &thread.to_string())?;
        }
        let dismissed = self
            .dismissed_suggestions
            .iter()
            .map(|ids| {
                ids.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join(";");
        self.store
            .set_state(&scope, "dismissed_suggestions", &dismissed)?;
        self.store.set_state(
            GLOBAL_SCOPE,
            "last_investigation",
            &workspace.id().to_string(),
        )?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        let Some(id) = self.workspace.as_ref().map(Workspace::id) else {
            return Ok(());
        };
        let data = self.store.load_investigation(id)?;
        self.workspace = Some(Workspace::new(data));
        self.touch_workspace();
        self.remap_focus_after_resolution();
        self.mark_dirty();
        Ok(())
    }

    fn apply_workspace_delta(&mut self, delta: WorkspaceDelta) -> Result<()> {
        if delta.is_empty() {
            return Ok(());
        }
        if self.workspace.is_none() {
            return self.reload();
        }
        if let Some(workspace) = self.workspace.as_mut() {
            workspace.apply_delta(delta);
        }
        self.touch_workspace();
        self.remap_focus_after_resolution();
        self.mark_dirty();
        Ok(())
    }

    fn touch_workspace(&mut self) {
        self.workspace_rev = self.workspace_rev.wrapping_add(1);
        self.clear_board_list_cache();
    }

    fn clear_board_list_cache(&mut self) {
        self.board_list_cache.replace(None);
    }

    fn track_job(&mut self, record: JobRecord) {
        self.job_records.insert(record.id, record);
        self.touch_workspace();
    }

    fn files(&self) -> Option<InvestigationFiles> {
        self.workspace
            .as_ref()
            .map(|workspace| InvestigationFiles::new(&self.paths, workspace.id()))
    }

    fn current_page(&self) -> Option<&Page> {
        let page_id = self.page_id?;
        self.workspace.as_ref()?.page(page_id)
    }

    fn current_target(&self) -> Option<Target> {
        self.current_page().and_then(Page::collection_target)
    }

    pub fn dispatch(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Quit => self.quit_or_close(),
            Action::Cancel => self.cancel(),
            Action::Tick => {
                for effect in self.motion.advance() {
                    match effect {
                        crate::tui::motion::TickEffect::Advanced
                        | crate::tui::motion::TickEffect::SplashDone
                        | crate::tui::motion::TickEffect::ToastExpired
                        | crate::tui::motion::TickEffect::ConfirmPulseDone
                        | crate::tui::motion::TickEffect::GraphRevealDone
                        | crate::tui::motion::TickEffect::DrawerDone
                        | crate::tui::motion::TickEffect::FocusDone => {}
                    }
                    debug_assert!(!effect.writes());
                }
                if self.motion.needs_redraw() {
                    self.mark_dirty();
                }
            }
            Action::NextPane => self.cycle_pane(1),
            Action::PrevPane => self.cycle_pane(-1),
            Action::Move(delta) => self.move_selection(delta),
            Action::Select => self.select()?,
            Action::Back => self.cancel(),
            Action::Help => {
                if matches!(self.overlay, Overlay::ThemeCycle { .. }) {
                    self.revert_theme_cycle();
                }
                self.scroll = 0;
                self.overlay = Overlay::Help;
            }
            Action::Palette => {
                self.overlay = Overlay::Palette {
                    query: Input::default(),
                    selected: 0,
                }
            }
            Action::Search => self.open_search_or_filter(),
            Action::ClearBoard => self.clear_board(),
            Action::AskAgent => {
                self.require_open_write()?;
                self.open_agent_prompt();
            }
            Action::NewItem => {
                if self.workspace.is_some() && self.view != View::Welcome {
                    self.require_open_write()?;
                }
                self.open_new()?;
            }
            Action::Edit => {
                if self.view != View::Settings && self.workspace.is_some() {
                    self.require_open_write()?;
                }
                self.open_edit()?;
            }
            Action::Refresh => {
                self.require_open_write()?;
                self.run_default_collectors()?;
            }
            Action::OpenJobs => self.switch_view(View::Jobs),
            Action::OpenTools => self.switch_view(View::Tools),
            Action::OpenSettings => self.switch_view(View::Settings),
            Action::OpenPages => self.switch_view(View::Overview),
            Action::OpenSources => self.switch_view(View::Evidence),
            Action::OpenFindings => self.switch_view(View::Evidence),
            Action::OpenEntities => self.switch_view(View::Entities),
            Action::OpenTimeline => {
                self.time_lens = true;
                self.switch_view(View::Evidence);
            }
            Action::OpenAgent => self.open_agent_history(),
            Action::OpenOverview => self.switch_view(View::Overview),
            Action::OpenEvidence => self.switch_view(View::Evidence),
            Action::OpenReview => self.open_review_lane(),
            Action::ToggleTimeLens => self.toggle_time_lens(),
            Action::ToggleZoom => {
                self.board_zoomed = !self.board_zoomed;
                self.status = if self.board_zoomed {
                    "Board zoomed. Any key returns.".into()
                } else {
                    "Board restored.".into()
                };
            }
            Action::Unzoom => {
                self.board_zoomed = false;
            }
            Action::OpenObjectView => self.open_object_view(),
            Action::OpenGraph => {
                self.merge_marks.clear();
                if self.view == View::Graph {
                    self.recenter_graph();
                } else {
                    self.open_graph();
                }
            }
            Action::SearchAround => self.search_around_from_selection(),
            Action::GraphHops(delta) => self.change_graph_hops(delta),
            Action::GraphFacetMove(delta) => self.move_graph_facet(delta),
            Action::GraphApplyFacet => self.apply_graph_facet(),
            Action::GraphClearFilters => self.clear_graph_filters(),
            Action::TimelineFacetMove(delta) => self.move_timeline_facet(delta),
            Action::TimelineApplyFilter => self.apply_timeline_filter(),
            Action::TimelineClearFilter => self.clear_timeline_filter(),
            Action::ToggleNearby => self.toggle_nearby(),
            Action::ToggleMergeMark if self.view == View::Review => self.toggle_review_mark(),
            Action::ToggleMergeMark
            | Action::ProposeMerge
            | Action::Unresolve
            | Action::OpenSuggestions => {
                let _ = self.dispatch_resolution(&action);
            }
            Action::Command(id) => self.run_command(id)?,
            Action::Submit => self.submit()?,
            Action::Cycle(delta) => self.cycle_form(delta),
            Action::Accept => self.decide_proposal(ProposalStatus::Accepted)?,
            Action::Reject => self.decide_proposal(ProposalStatus::Rejected)?,
            Action::Arm => self.arm_review(),
            Action::Disarm => self.disarm_review(),
            Action::Job(event) => self.handle_job(event)?,
            Action::InsertChar(ch) => self.insert_char(ch),
            Action::Backspace => self.backspace(),
            Action::Delete => self.delete_char(),
            Action::Left => self.input_left(),
            Action::Right => self.input_right(),
            Action::Home => self.input_home(),
            Action::End => self.input_end(),
            Action::Scroll(delta) => {
                self.scroll = add_scroll(self.scroll, delta);
            }
            Action::OpenInvestigation(id) => self.open_investigation(id)?,
        }
        self.mark_dirty();
        Ok(())
    }

    fn quit_or_close(&mut self) {
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            return;
        }
        let _ = self.save_session();
        self.should_quit = true;
    }

    fn cancel(&mut self) {
        match &self.overlay {
            Overlay::None => {
                if self.view == View::Object {
                    self.close_object_view();
                    return;
                }
                if self.view == View::Graph {
                    self.close_graph();
                    return;
                }
                if self.cancel_timeline_mode() {
                    return;
                }
                if self.view != View::Welcome && self.workspace.is_none() {
                    self.view = View::Welcome;
                }
            }
            Overlay::Confirm { command, .. } => {
                if command == "graph_confirm_cap" {
                    self.graph.pending = None;
                }
                if command == "confirm_resolve" {
                    self.pending_resolve = None;
                }
                if command == "confirm_unresolve" {
                    self.pending_unresolve = None;
                }
                self.overlay = Overlay::None;
            }
            Overlay::ThemeCycle { .. } => self.revert_theme_cycle(),
            Overlay::Form(_)
            | Overlay::Palette { .. }
            | Overlay::Search { .. }
            | Overlay::TableFilter { .. }
            | Overlay::AgentPrompt { .. }
            | Overlay::Help
            | Overlay::Text { .. }
            | Overlay::Doctor => {
                self.overlay = Overlay::None;
            }
        }
    }

    fn cycle_pane(&mut self, delta: i32) {
        let previous = self.pane;
        self.pane = match (self.pane, delta.signum()) {
            (Pane::Nav, 1) | (Pane::Detail, -1) => Pane::Main,
            (Pane::Main, 1) => Pane::Detail,
            (Pane::Main, -1) => Pane::Nav,
            (Pane::Nav, -1) => Pane::Detail,
            (Pane::Detail, 1) => Pane::Nav,
            _ => self.pane,
        };
        if self.pane != previous {
            self.motion.start_focus();
        }
        if self.pane == Pane::Detail && matches!(self.view, View::Entities | View::Evidence) {
            self.inspector_tab = InspectorTab::Sources;
        }
    }

    fn canonicalize_view(view: View) -> View {
        match view {
            View::Pages | View::Notes => View::Overview,
            View::Sources | View::Findings | View::Timeline => View::Evidence,
            View::Agent => View::Review,
            other => other,
        }
    }

    pub fn is_time_lens(&self) -> bool {
        self.view == View::Timeline
            || (self.time_lens
                && matches!(self.view, View::Evidence | View::Graph | View::Entities))
    }

    fn toggle_time_lens(&mut self) {
        if !matches!(
            self.view,
            View::Evidence | View::Graph | View::Entities | View::Timeline
        ) {
            return;
        }
        self.time_lens = !self.time_lens;
        self.selected = 0;
        self.status = if self.time_lens {
            "Time lens on. + - change the bucket. t toggles off.".into()
        } else {
            "Time lens off.".into()
        };
    }

    pub(crate) fn view_uses_table_filter(view: View) -> bool {
        matches!(
            view,
            View::Entities | View::Evidence | View::Jobs | View::Tools
        )
    }

    fn open_search_or_filter(&mut self) {
        if Self::view_uses_table_filter(self.view) && !self.is_time_lens() {
            self.overlay = Overlay::TableFilter {
                query: Input::new(self.board_filter.clone()),
            };
            self.status = "Filter: kind:ip src:ct prov:ai   Enter apply   x clear".into();
            return;
        }
        self.overlay = Overlay::Search {
            query: Input::default(),
            selected: 0,
        };
    }

    fn clear_board(&mut self) {
        if self.motion.dismiss_failed_toasts() {
            self.status = "Toast dismissed.".into();
            self.mark_dirty();
            return;
        }
        let had_error = self.last_error.take();
        let had_filter = !self.board_filter.is_empty();
        self.board_filter.clear();
        if matches!(self.overlay, Overlay::TableFilter { .. }) {
            self.overlay = Overlay::None;
        }
        if self.view == View::Graph {
            self.clear_graph_filters();
        } else if self.is_time_lens() {
            self.clear_timeline_filter();
        }
        if had_error.is_some() {
            self.status = "Error dismissed.".into();
        } else if had_filter {
            self.status = "Filter cleared.".into();
            self.restore_board_selection();
        }
    }

    /// Animate the Inspector drawer only when the layout crosses into the narrow breakpoint.
    pub fn on_resize(&mut self, narrow: bool) {
        if narrow && !self.drawer_narrow && !self.board_zoomed {
            self.motion.start_drawer();
        }
        self.drawer_narrow = narrow;
    }

    pub fn active_board_filter(&self) -> &str {
        if let Overlay::TableFilter { query } = &self.overlay {
            query.value.as_str()
        } else {
            self.board_filter.as_str()
        }
    }

    pub fn parsed_board_filter(&self) -> filter::TableFilter {
        filter::TableFilter::parse(self.active_board_filter())
    }

    pub fn effective_sort(&self) -> Option<(filter::SortKey, bool)> {
        self.parsed_board_filter().sort
    }

    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.last_error = Some(message.clone());
        self.status = message;
    }

    fn select_tool_row(&mut self) {
        let Some(entry) = self
            .listed_tool_indices()
            .get(self.selected)
            .and_then(|index| self.tools.tools().get(*index))
            .cloned()
        else {
            return;
        };
        let name = entry.definition.name.clone();
        let refused = entry.definition.activity == Activity::Active
            || entry
                .problem
                .as_deref()
                .is_some_and(|problem| problem.contains("refused"));
        if refused {
            let reason = entry
                .problem
                .clone()
                .unwrap_or_else(|| "os-int observes public information only".into());
            let mark = if self.config.ui.ascii { "no" } else { "⊘" };
            self.set_error(format!(
                "{mark} {name}  {}  refused: {reason}",
                entry.definition.activity.as_str()
            ));
            self.motion.push_toast(
                crate::tui::motion::ToastKind::Refused,
                format!("active tool refused: {name} — {reason}"),
            );
            return;
        }
        self.overlay = Overlay::Confirm {
            title: format!("Run {name}"),
            message: format!(
                "Run {name}? This writes case data. Press y to confirm. Enter never runs a tool."
            ),
            command: format!("run_tool:{name}"),
        };
    }

    fn switch_view(&mut self, view: View) {
        if self.workspace.is_none() && !matches!(view, View::Welcome | View::Settings | View::Tools)
        {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        if view != View::Review {
            self.merge_marks.clear();
        }
        self.review_armed = false;
        self.review_marks.clear();
        if !matches!(
            view,
            View::Evidence | View::Timeline | View::Graph | View::Entities
        ) {
            self.time_lens = false;
        }
        if !matches!(
            view,
            View::Timeline | View::Entities | View::Evidence | View::Graph
        ) {
            self.timeline.nearby = false;
            self.timeline.nearby_seed_id = None;
        }
        self.board_filter.clear();
        self.board_anchor_id = None;
        self.inspector_tab = if matches!(view, View::Entities | View::Evidence) {
            InspectorTab::Sources
        } else {
            InspectorTab::Props
        };
        if view == View::Graph {
            self.open_graph();
            self.pane = Pane::Main;
            if self.time_lens {
                self.status = self.view_status_line();
            }
            let _ = self.save_session();
            return;
        }
        self.view = view;
        self.selected = 0;
        self.scroll = 0;
        self.pane = Pane::Main;
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == view)
        {
            self.nav_index = index;
        }
        self.overlay = Overlay::None;
        self.sync_board_selection();
        self.status = self.view_status_line();
        let _ = self.save_session();
    }

    fn view_status_line(&self) -> String {
        match self.view {
            View::Review => {
                let count = self.pending_review_count();
                if count == 0 {
                    "Review is empty. Agent, playbook, and merge proposals land here.".into()
                } else {
                    format!("{count} item(s) waiting. Shift+A then y writes. Enter never writes.")
                }
            }
            View::Evidence => "Evidence. t toggles the time lens. Enter never writes.".into(),
            View::Graph => "Graph. t toggles the time lens. s Search Around.".into(),
            View::Entities => {
                "Entities. t toggles the time lens. Space marks. m proposes a merge.".into()
            }
            View::Jobs => "Jobs. Enter opens the selected job. Cancel is in the palette.".into(),
            View::Tools => "Tools. Enter opens a y-confirm. Active tools stay refused.".into(),
            other => format!("{} — Enter opens. It never writes.", other.title()),
        }
    }

    pub fn rail_view_is_current(&self, view: View) -> bool {
        if self
            .nav_items()
            .iter()
            .any(|(_, candidate)| *candidate == self.view)
        {
            return self.view == view;
        }
        self.nav_items()
            .get(self.nav_index)
            .is_some_and(|(_, candidate)| *candidate == view)
    }

    fn move_selection(&mut self, delta: i32) {
        let search_count = if let Overlay::Search { query, .. } = &self.overlay {
            Some(self.search_hits(&query.value).len().max(1))
        } else {
            None
        };
        match &mut self.overlay {
            Overlay::ThemeCycle { index, .. } => {
                let count = Self::THEME_CYCLE.len() as i32;
                *index = (*index as i32 + delta).rem_euclid(count) as usize;
                let preview = *index;
                self.apply_theme_preview(preview);
            }
            Overlay::Palette { selected, query } => {
                let count = filtered(&query.value).len().max(1);
                *selected = (*selected as i32 + delta).rem_euclid(count as i32) as usize;
            }
            Overlay::Search { selected, .. } => {
                let count = search_count.unwrap_or(1);
                *selected = (*selected as i32 + delta).rem_euclid(count as i32) as usize;
            }
            Overlay::Form(form) => {
                let count = form.fields.len().max(1);
                form.index = (form.index as i32 + delta).rem_euclid(count as i32) as usize;
            }
            Overlay::Text { scroll, .. } => {
                *scroll = add_scroll(*scroll, delta);
            }
            Overlay::Help => {
                self.scroll = add_scroll(self.scroll, delta);
            }
            Overlay::None if self.view == View::Welcome => {
                const WELCOME_CHOICES: i32 = 4;
                self.selected = (self.selected as i32 + delta).rem_euclid(WELCOME_CHOICES) as usize;
            }
            Overlay::None
            | Overlay::TableFilter { .. }
            | Overlay::AgentPrompt { .. }
            | Overlay::Confirm { .. }
            | Overlay::Doctor => {
                if self.pane == Pane::Nav {
                    let count = self.nav_items().len().max(1);
                    self.nav_index =
                        (self.nav_index as i32 + delta).rem_euclid(count as i32) as usize;
                } else if self.pane == Pane::Detail {
                    self.cycle_inspector_tab(delta);
                } else if self.view == View::Object {
                    self.scroll = add_scroll(self.scroll, delta);
                } else if self.view == View::Graph && !self.time_lens {
                    self.move_graph_selection(delta);
                } else if self.view == View::Timeline
                    || (self.time_lens
                        && matches!(self.view, View::Evidence | View::Graph | View::Entities))
                {
                    let count = self.timeline_list_len().max(1);
                    self.selected =
                        (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
                    self.sync_timeline_selection();
                } else if self.view == View::Review {
                    let count = self.listed_review_items().len().max(1);
                    self.selected =
                        (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
                    self.sync_board_selection();
                } else {
                    let count = self.board_len();
                    if count == 0 {
                        self.selected = 0;
                        self.sync_board_selection();
                    } else {
                        self.selected =
                            (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
                        self.sync_board_selection();
                    }
                }
            }
        }
    }

    fn board_len(&self) -> usize {
        match self.view {
            View::Overview | View::Pages => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.data.pages.len())
                .unwrap_or(0),
            View::Entities if self.time_lens => self.timeline_list_len(),
            View::Entities => self.listed_entities().len(),
            View::Evidence if self.time_lens => self.timeline_list_len(),
            View::Evidence => self.listed_evidence().len(),
            View::Jobs => self.listed_job_ids().len(),
            View::Tools => self.listed_tool_indices().len(),
            View::Findings => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.data.findings.len())
                .unwrap_or(0),
            View::Sources => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.data.sources.len())
                .unwrap_or(0),
            View::Artifacts => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.data.artifacts.len())
                .unwrap_or(0),
            View::Audit => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.data.audit.len())
                .unwrap_or(0),
            _ => 0,
        }
    }

    pub(crate) fn sync_board_selection(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        match self.view {
            View::Overview | View::Pages => {
                if let Some(page) = workspace.data.pages.get(self.selected) {
                    self.page_id = Some(page.id);
                    self.entity_id = page
                        .entity_id
                        .or_else(|| workspace.page_entity_ids(page.id).into_iter().next());
                }
            }
            View::Entities if self.time_lens => self.sync_timeline_selection(),
            View::Entities => {
                let selected_id = self
                    .listed_entities()
                    .get(self.selected)
                    .map(|entity| entity.id);
                if let Some(id) = selected_id {
                    self.entity_id = Some(id);
                    self.board_anchor_id = Some(id);
                } else {
                    self.entity_id = None;
                    self.finding_id = None;
                    self.source_id = None;
                }
            }
            View::Evidence if self.time_lens => self.sync_timeline_selection(),
            View::Evidence => {
                let row = self.listed_evidence().get(self.selected).copied();
                match row {
                    Some(EvidenceRow::Finding(id)) => {
                        self.finding_id = Some(id);
                        self.source_id = None;
                        self.board_anchor_id = Some(id);
                        if let Some(page_id) =
                            workspace.finding(id).and_then(|finding| finding.page_id)
                        {
                            self.page_id = Some(page_id);
                        }
                    }
                    Some(EvidenceRow::Source(id)) => {
                        self.source_id = Some(id);
                        self.finding_id = None;
                        self.board_anchor_id = Some(id);
                    }
                    None => {
                        self.entity_id = None;
                        self.finding_id = None;
                        self.source_id = None;
                    }
                }
            }
            View::Review => {
                if let Some(run_id) = self.selected_review_item().and_then(|item| item.run_id) {
                    self.agent_run_id = Some(run_id);
                }
            }
            View::Findings => {
                if let Some(finding) = workspace.data.findings.get(self.selected) {
                    self.finding_id = Some(finding.id);
                }
            }
            View::Sources => {
                if let Some(source) = workspace.data.sources.get(self.selected) {
                    self.source_id = Some(source.id);
                }
            }
            View::Graph if !self.time_lens => {}
            View::Graph => self.sync_timeline_selection(),
            _ => {}
        }
    }

    fn select(&mut self) -> Result<()> {
        match &self.overlay {
            Overlay::ThemeCycle { .. } => {
                self.save_theme_cycle()?;
            }
            Overlay::Palette { query, selected } => {
                let items = filtered(&query.value);
                if let Some(command) = items.get(*selected).copied() {
                    if command.id == "theme" {
                        self.overlay = Overlay::None;
                        self.run_command(command.id)?;
                    } else if command.writes {
                        self.overlay = Overlay::Confirm {
                            title: command.title.to_string(),
                            message: format!(
                                "Run {}? This writes case data. Press y to confirm.",
                                command.title
                            ),
                            command: command.id.to_string(),
                        };
                    } else {
                        self.overlay = Overlay::None;
                        self.run_command(command.id)?;
                    }
                }
            }
            Overlay::Search { query, selected } => {
                let hits = self.search_hits(&query.value);
                if let Some(hit) = hits.get(*selected).cloned() {
                    self.overlay = Overlay::None;
                    self.open_hit(&hit);
                }
            }
            Overlay::TableFilter { query } => {
                self.board_filter = query.value.clone();
                self.overlay = Overlay::None;
                self.restore_board_selection();
            }
            Overlay::None if self.view == View::Welcome => match self.selected {
                0 => self.open_new()?,
                1 => self.open_open_form(),
                2 => self.overlay = Overlay::Doctor,
                _ => self.overlay = Overlay::Help,
            },
            Overlay::None if self.pane == Pane::Nav => {
                let view = self.nav_items()[self.nav_index].1;
                if view == View::Review {
                    self.open_review_lane();
                } else {
                    self.switch_view(view);
                }
            }
            Overlay::None => self.select_main_item(),
            Overlay::Confirm { command, .. } => {
                let command = command.clone();
                self.overlay = Overlay::None;
                self.run_command(&command)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn cycle_inspector_tab(&mut self, delta: i32) {
        let tabs = InspectorTab::ALL;
        let index = tabs
            .iter()
            .position(|tab| *tab == self.inspector_tab)
            .unwrap_or(0);
        let next = (index as i32 + delta).rem_euclid(tabs.len() as i32) as usize;
        self.inspector_tab = tabs[next];
    }

    fn select_main_item(&mut self) {
        if self.view == View::Review {
            self.expand_review_item();
            return;
        }
        if self.view == View::Tools {
            self.select_tool_row();
            return;
        }
        if self.view == View::Jobs {
            self.expand_selected_job();
            return;
        }
        let listed_entity_id = if self.view == View::Entities {
            self.listed_entities()
                .get(self.selected)
                .map(|entity| entity.id)
        } else {
            None
        };
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        match self.view {
            View::Overview | View::Pages => {
                if let Some(page) = workspace.data.pages.get(self.selected) {
                    self.page_id = Some(page.id);
                    self.entity_id = page
                        .entity_id
                        .or_else(|| workspace.page_entity_ids(page.id).into_iter().next());
                    let _ = self.save_session();
                    self.status = format!("Target {}", page.title);
                }
            }
            View::Evidence if self.time_lens => self.select_timeline_row(),
            View::Evidence => {
                if let Some(row) = self.listed_evidence().get(self.selected).cloned() {
                    match row {
                        EvidenceRow::Finding(id) => self.finding_id = Some(id),
                        EvidenceRow::Source(id) => {
                            self.source_id = Some(id);
                            if let Some(source) = workspace.source(id) {
                                self.overlay = Overlay::Text {
                                    title: format!("{} {}", source.short_ref(), source.title),
                                    body: source.content.clone(),
                                    scroll: 0,
                                };
                            }
                        }
                    }
                }
            }
            View::Sources => {
                if let Some(source) = workspace.data.sources.get(self.selected) {
                    self.source_id = Some(source.id);
                    self.overlay = Overlay::Text {
                        title: format!("{} {}", source.short_ref(), source.title),
                        body: source.content.clone(),
                        scroll: 0,
                    };
                }
            }
            View::Entities => {
                if let Some(id) = listed_entity_id {
                    self.entity_id = Some(id);
                    self.open_object_view();
                }
            }
            View::Graph => self.open_selected_graph_node(),
            View::Timeline => self.select_timeline_row(),
            View::Findings => {
                if let Some(finding) = workspace.data.findings.get(self.selected) {
                    self.finding_id = Some(finding.id);
                }
            }
            View::Agent => {
                if let Some(run) = workspace.data.agent_runs.get(self.selected) {
                    self.agent_run_id = Some(run.id);
                    self.agent_thread = Some(run.thread_id);
                }
            }
            View::Artifacts => {
                if let Some(artifact) = workspace.data.artifacts.get(self.selected).cloned() {
                    self.open_artifact(&artifact);
                }
            }
            _ => {}
        }
    }

    fn expand_review_item(&mut self) {
        let item = self.selected_review_item();
        let run = {
            let workspace = self.workspace.as_ref();
            let run_id = item
                .as_ref()
                .and_then(|item| item.run_id)
                .or(self.agent_run_id);
            workspace.and_then(|workspace| run_id.and_then(|id| workspace.agent_run(id).cloned()))
        };
        if let Some(item) = item {
            let mut body = item.preview.join("\n");
            if !item.citations.is_empty() {
                body.push_str("\n\nCited evidence\n");
                for citation in &item.citations {
                    let mark = if citation.present { "ok" } else { "missing" };
                    body.push_str(&format!(
                        "  {mark} {}  {}\n",
                        citation.reference, citation.label
                    ));
                }
            }
            if !item.rationale.is_empty() {
                body.push_str("\nRationale\n");
                body.push_str(&item.rationale);
            }
            if let Some(run) = &run {
                body.push_str("\n\nTranscript\n");
                body.push_str(&format!("Q  {}\n\n{}", run.question, run.response));
            }
            self.overlay = Overlay::Text {
                title: item.title,
                body,
                scroll: 0,
            };
            self.status = "Expanded. Shift+A then y writes. Enter never writes.".into();
            return;
        }
        if let Some(run) = run {
            self.overlay = Overlay::Text {
                title: format!("{} {}", run.short_ref(), run.agent_name),
                body: format!("Q  {}\n\n{}", run.question, run.response),
                scroll: 0,
            };
            self.status = "Agent transcript. Nothing to accept.".into();
            return;
        }
        self.status = "Nothing to expand. Shift+A then y writes. Enter never writes.".into();
    }

    fn expand_selected_job(&mut self) {
        let Some(job_id) = self.listed_job_ids().get(self.selected).copied() else {
            return;
        };
        let stored = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.job(job_id));
        let live = self.job_records.get(&job_id);
        let title = live
            .map(|job| job.description.clone())
            .or_else(|| stored.map(|job| job.description.clone()))
            .unwrap_or_else(|| job_id.to_string());
        let reference = stored
            .map(|job| job.short_ref().to_string())
            .or_else(|| live.map(|job| job.short_ref().to_string()))
            .unwrap_or_default();
        let body = format!(
            "Program: {}\nArguments: {}\nStatus: {}\n\n{}\n\n{}",
            live.and_then(|job| job.program.clone())
                .or_else(|| stored.and_then(|job| job.program.clone()))
                .unwrap_or_else(|| "-".into()),
            live.map(|job| job.args.join(" "))
                .or_else(|| stored.map(|job| job.args.join(" ")))
                .unwrap_or_default(),
            live.map(|job| job.status.to_string())
                .or_else(|| stored.map(|job| job.status.to_string()))
                .unwrap_or_else(|| "-".into()),
            live.map(|job| job.log.as_str())
                .or_else(|| stored.map(|job| job.log.as_str()))
                .unwrap_or(""),
            live.map(|job| job.stderr.as_str())
                .or_else(|| stored.map(|job| job.stderr.as_str()))
                .unwrap_or("")
        );
        self.overlay = Overlay::Text {
            title: format!("{reference} {title}"),
            body,
            scroll: 0,
        };
    }

    fn open_hit(&mut self, hit: &SearchHit) {
        self.switch_view(hit.view);
        match hit.view {
            View::Pages | View::Overview => self.page_id = Some(hit.id),
            View::Sources | View::Evidence => {
                if hit.kind == "finding" {
                    self.finding_id = Some(hit.id);
                } else {
                    self.source_id = Some(hit.id);
                }
            }
            View::Findings => self.finding_id = Some(hit.id),
            View::Entities => self.entity_id = Some(hit.id),
            View::Agent | View::Review => {
                self.agent_run_id = Some(hit.id);
                if let Some(thread_id) = self
                    .workspace
                    .as_ref()
                    .and_then(|workspace| workspace.agent_run(hit.id))
                    .map(|run| run.thread_id)
                {
                    self.agent_thread = Some(thread_id);
                }
                if let Some(index) = self
                    .listed_review_items()
                    .iter()
                    .position(|item| item.run_id == Some(hit.id))
                {
                    self.selected = index;
                }
            }
            View::Artifacts => {
                if let Some(index) = self.workspace.as_ref().and_then(|workspace| {
                    workspace
                        .data
                        .artifacts
                        .iter()
                        .position(|artifact| artifact.id == hit.id)
                }) {
                    self.selected = index;
                }
            }
            _ => {}
        }
    }

    fn open_new(&mut self) -> Result<()> {
        if self.workspace.is_none() || self.view == View::Welcome {
            self.overlay = Overlay::Form(FormState::new(
                FormKind::NewInvestigation,
                vec![
                    field("Name", "", "Operation Example"),
                    field("Description", "", "optional"),
                ],
            ));
            return Ok(());
        }
        match self.view {
            View::Findings | View::Evidence => self.overlay = Overlay::Form(finding_form()),
            View::Notes => self.overlay = Overlay::Form(note_form()),
            _ => self.overlay = Overlay::Form(page_form()),
        }
        Ok(())
    }

    fn open_open_form(&mut self) {
        let names = self
            .investigations()
            .into_iter()
            .map(|summary| summary.investigation.name)
            .collect::<Vec<_>>()
            .join(", ");
        self.overlay = Overlay::Form(FormState::new(
            FormKind::OpenInvestigation,
            vec![field("Name or ID", "", &names)],
        ));
    }

    fn open_edit(&mut self) -> Result<()> {
        if self.inspector_tab == InspectorTab::Notes
            && self.workspace.is_some()
            && let Some(entity) = self
                .selected_entity_id()
                .and_then(|id| self.workspace.as_ref()?.entity(id))
                .cloned()
        {
            return self.open_entity_notes_editor(&entity);
        }
        if self.view == View::Settings {
            let form = FormState::new(
                FormKind::SettingsAgent,
                vec![
                    field(
                        "Active profile",
                        &self.config.agent.active,
                        "claude / codex / custom",
                    ),
                    field(
                        "Program",
                        self.config.agent.program.as_deref().unwrap_or(""),
                        "claude",
                    ),
                    field(
                        "Timeout seconds",
                        &self.config.agent.timeout_seconds.to_string(),
                        "600",
                    ),
                    field(
                        "Theme",
                        &self.config.ui.theme,
                        "auto / dark / light / mono / phosphor / amber",
                    ),
                ],
            );
            self.overlay = Overlay::Form(form);
            return Ok(());
        }
        if self.workspace.is_some()
            && let Some(page) = self.current_page().cloned()
        {
            return self.open_external_editor(&page);
        }
        if let Some(page) = self.current_page() {
            self.overlay = Overlay::Form(FormState::new(
                FormKind::NewNote,
                vec![field("Notes", &page.notes, "Markdown")],
            ));
        } else {
            self.status = "Select a target first, then press e to edit notes in $EDITOR.".into();
        }
        Ok(())
    }

    fn open_external_editor(&mut self, page: &Page) -> Result<()> {
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        files.ensure().ok();
        let relative = format!("notes/edit-{}.md", page.id.simple());
        let path = files
            .resolve(&relative)
            .ok_or_else(|| eyre!("unsafe notes path"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).wrap_err("could not create notes directory")?;
        }
        std::fs::write(&path, &page.notes).wrap_err("could not write notes for editing")?;
        self.pending_editor = Some(EditorSession {
            path,
            page_id: Some(page.id),
            entity_id: None,
        });
        self.status = "Opening editor…".into();
        Ok(())
    }

    fn open_entity_notes_editor(&mut self, entity: &crate::model::Entity) -> Result<()> {
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        files.ensure().ok();
        let relative = format!("notes/entity-{}.md", entity.id.simple());
        let path = files
            .resolve(&relative)
            .ok_or_else(|| eyre!("unsafe notes path"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).wrap_err("could not create notes directory")?;
        }
        std::fs::write(&path, &entity.notes).wrap_err("could not write notes for editing")?;
        self.pending_editor = Some(EditorSession {
            path,
            page_id: None,
            entity_id: Some(entity.id),
        });
        self.status = "Opening editor…".into();
        Ok(())
    }

    pub fn take_pending_editor(&mut self) -> Option<EditorSession> {
        self.pending_editor.take()
    }

    pub fn apply_editor_session(&mut self, session: EditorSession) -> Result<()> {
        let notes = std::fs::read_to_string(&session.path).unwrap_or_default();
        let _ = std::fs::remove_file(&session.path);
        if let Some(entity_id) = session.entity_id {
            let label = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(entity_id))
                .map(|entity| entity.label.clone())
                .unwrap_or_default();
            self.store.update_entity(entity_id, &label, &notes)?;
            self.reload()?;
            self.status = "Entity notes updated.".into();
            return Ok(());
        }
        let Some(page_id) = session.page_id else {
            return Ok(());
        };
        self.store.update_page_notes(page_id, &notes)?;
        if let Some(page) = self.store.load_page(page_id)? {
            self.apply_workspace_delta(WorkspaceDelta {
                pages: vec![page],
                ..WorkspaceDelta::default()
            })?;
        } else {
            self.reload()?;
        }
        self.status = "Notes updated.".into();
        Ok(())
    }

    pub fn editor_command(&self) -> Vec<String> {
        let raw = self
            .config
            .ui
            .editor
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("VISUAL")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| "vi".into());
        shlex::split(&raw)
            .filter(|parts| !parts.is_empty())
            .unwrap_or_else(|| vec![raw])
    }

    fn open_artifact(&mut self, artifact: &Artifact) {
        let body = self
            .files()
            .and_then(|files| files.read_text(&artifact.path, 200_000).ok())
            .unwrap_or_else(|| {
                format!(
                    "{}\n{}\n{} bytes\n{}",
                    artifact.name,
                    artifact.path,
                    artifact.size_bytes,
                    artifact.sha256.as_deref().unwrap_or("-")
                )
            });
        self.overlay = Overlay::Text {
            title: format!("{} {}", artifact.short_ref(), artifact.name),
            body,
            scroll: 0,
        };
    }

    fn open_agent_prompt(&mut self) {
        if self.workspace.is_none() {
            self.status = "Open an investigation before asking the agent.".into();
            return;
        }
        if !self.agent.available {
            self.status = self
                .agent
                .message
                .clone()
                .unwrap_or_else(|| "No agent on PATH.".into());
            self.overlay = Overlay::Text {
                title: "Agent unavailable".into(),
                body: self.status.clone(),
                scroll: 0,
            };
            return;
        }
        self.overlay = Overlay::AgentPrompt {
            input: Input::default(),
        };
    }

    fn submit(&mut self) -> Result<()> {
        match &self.overlay {
            Overlay::AgentPrompt { input } => {
                let question = input.value.trim().to_string();
                if question.is_empty() {
                    return Ok(());
                }
                self.overlay = Overlay::None;
                self.ask_agent(&question)?;
            }
            Overlay::Form(form)
                if matches!(
                    form.kind,
                    FormKind::Ingest
                        | FormKind::Playbook
                        | FormKind::PlaybookPrompt
                        | FormKind::Watch
                ) =>
            {
                let form = form.clone();
                self.submit_ingest_form(form)?;
            }
            Overlay::Form(form) => {
                let form = form.clone();
                self.overlay = Overlay::None;
                self.submit_form(form)?;
            }
            Overlay::Palette { .. } | Overlay::Search { .. } => self.select()?,
            _ => {}
        }
        Ok(())
    }

    fn submit_form(&mut self, form: FormState) -> Result<()> {
        let values: Vec<String> = form
            .fields
            .iter()
            .map(|field| field.value.value.trim().to_string())
            .collect();
        match form.kind {
            FormKind::NewInvestigation => {
                let investigation = self
                    .store
                    .create_investigation(&values[0], values.get(1).unwrap_or(&String::new()))?;
                InvestigationFiles::new(&self.paths, investigation.id)
                    .ensure()
                    .wrap_err("could not create investigation directory")?;
                self.open_investigation(investigation.id)?;
            }
            FormKind::OpenInvestigation => {
                let found = self.store.require_investigation(&values[0])?;
                self.open_investigation(found.id)?;
            }
            FormKind::NewPage => {
                let workspace = self
                    .workspace
                    .as_ref()
                    .ok_or_else(|| eyre!("no investigation"))?;
                let kind = PageKind::parse(values.get(1).map(String::as_str).unwrap_or("generic"));
                let target = values
                    .get(2)
                    .filter(|value| !value.is_empty())
                    .map(String::as_str);
                let page = self.store.create_page(NewPage::new(
                    workspace.id(),
                    &values[0],
                    kind,
                    target,
                ))?;
                self.page_id = Some(page.id);
                let mut delta = WorkspaceDelta {
                    pages: vec![page.clone()],
                    ..WorkspaceDelta::default()
                };
                if let Some(entity_id) = page.entity_id {
                    delta.page_entities.push(PageEntityLink {
                        page_id: page.id,
                        entity_id,
                    });
                    if let Some(entity) = self.store.load_entity(entity_id)? {
                        delta.entities.push(entity);
                    }
                }
                self.apply_workspace_delta(delta)?;
                self.view = View::Overview;
                self.status = format!("Created {} — press c to collect", page.title);
            }
            FormKind::NewFinding => {
                let workspace = self
                    .workspace
                    .as_ref()
                    .ok_or_else(|| eyre!("no investigation"))?;
                let confidence =
                    Confidence::parse(values.get(2).map(String::as_str).unwrap_or("medium"))
                        .unwrap_or(Confidence::Medium);
                let sources = crate::model::parse_ref_list(values.get(3).unwrap_or(&String::new()))
                    .unwrap_or_default();
                let source_ids: Vec<Uuid> = sources
                    .into_iter()
                    .filter_map(|reference| workspace.resolve_ref(reference))
                    .collect();
                let finding = self.store.create_finding(NewFinding {
                    investigation_id: workspace.id(),
                    page_id: self.page_id,
                    title: values[0].clone(),
                    details: values.get(1).cloned().unwrap_or_default(),
                    confidence,
                    provenance: Provenance::Analyst,
                    source_ids,
                })?;
                self.apply_workspace_delta(WorkspaceDelta {
                    findings: vec![finding],
                    ..WorkspaceDelta::default()
                })?;
                self.status = "Finding added.".into();
            }
            FormKind::NewNote => {
                if self.view == View::Overview || self.current_page().is_some() {
                    if let Some(page_id) = self.page_id {
                        self.store
                            .update_page_notes(page_id, values.first().unwrap_or(&String::new()))?;
                    }
                } else if let Some(workspace) = &self.workspace {
                    let note = self.store.create_note(
                        workspace.id(),
                        self.page_id,
                        values.first().unwrap_or(&String::new()),
                        Provenance::Analyst,
                    )?;
                    self.apply_workspace_delta(WorkspaceDelta {
                        notes: vec![note],
                        ..WorkspaceDelta::default()
                    })?;
                }
                if (self.view == View::Overview || self.current_page().is_some())
                    && let Some(page_id) = self.page_id
                    && let Some(page) = self.store.load_page(page_id)?
                {
                    self.apply_workspace_delta(WorkspaceDelta {
                        pages: vec![page],
                        ..WorkspaceDelta::default()
                    })?;
                }
            }
            FormKind::Ingest | FormKind::Playbook | FormKind::PlaybookPrompt | FormKind::Watch => {
                self.submit_ingest_form(form)?;
            }
            FormKind::Marking => {
                let workspace = self
                    .workspace
                    .as_ref()
                    .ok_or_else(|| eyre!("no investigation"))?;
                let marking =
                    crate::model::Marking::parse(values.first().unwrap_or(&String::new()))
                        .map_err(|error| eyre!("{error}"))?;
                let investigation_id = workspace.id();
                self.store.set_marking(investigation_id, marking)?;
                let investigation = self
                    .store
                    .get_investigation(investigation_id)?
                    .ok_or_else(|| eyre!("investigation disappeared"))?;
                self.apply_workspace_delta(WorkspaceDelta {
                    investigation: Some(investigation),
                    audit: self.store.list_audit(investigation_id)?,
                    ..WorkspaceDelta::default()
                })?;
                self.status = format!("Marking set to {}", marking.label());
            }
            FormKind::TeamMember => {
                let workspace = self
                    .workspace
                    .as_ref()
                    .ok_or_else(|| eyre!("no investigation"))?;
                let user_id = values.first().cloned().unwrap_or_default();
                let role = crate::model::Role::parse(values.get(1).unwrap_or(&String::new()))
                    .map_err(|error| eyre!("{error}"))?;
                let investigation_id = workspace.id();
                self.store.add_member(investigation_id, &user_id, role)?;
                self.apply_workspace_delta(WorkspaceDelta {
                    members: self.store.list_members(investigation_id)?,
                    ..WorkspaceDelta::default()
                })?;
                self.status = format!("Added {user_id} as {}", role.as_str());
            }
            FormKind::SettingsAgent => {
                self.config.agent.active = values[0].clone();
                self.config.agent.program =
                    values.get(1).filter(|value| !value.is_empty()).cloned();
                if let Ok(timeout) = values.get(2).map(String::as_str).unwrap_or("").parse() {
                    self.config.agent.timeout_seconds = timeout;
                }
                if let Some(theme) = values.get(3).filter(|value| !value.is_empty()) {
                    let known = theme.eq_ignore_ascii_case("auto")
                        || crate::tui::ThemeName::parse(theme).is_some();
                    if !known {
                        return Err(eyre!(
                            "unknown theme {theme:?}; valid themes: auto, dark, light, mono, phosphor, amber"
                        ));
                    }
                    self.config.ui.theme = theme.clone();
                }
                self.config.save(&self.paths.config_file)?;
                self.agent = AgentStatus::resolve(&self.config, &self.tools);
                self.status = format!("Agent set to {}", self.agent.profile.name);
            }
        }
        Ok(())
    }

    fn command_requires_write(id: &str) -> bool {
        commands::command_writes(id)
    }

    fn require_open_write(&self) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("Create or open an investigation first."))?;
        self.store.assert_can_mutate(workspace.id())
    }

    fn require_open_marking(&self) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("Create or open an investigation first."))?;
        self.store.assert_can_set_marking(workspace.id())
    }

    fn require_open_team(&self) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("Create or open an investigation first."))?;
        self.store.assert_can_manage_team(workspace.id())
    }

    fn run_command(&mut self, id: &str) -> Result<()> {
        if id == "set_marking" {
            self.require_open_marking()?;
        } else if id == "add_member" {
            self.require_open_team()?;
        } else if id != "new_investigation" && id != "theme" && Self::command_requires_write(id) {
            self.require_open_write()?;
        }
        match id {
            "new_investigation" => {
                self.workspace = None;
                self.view = View::Welcome;
                self.open_new()?;
            }
            "open_investigation" => self.open_open_form(),
            "new_page" => self.overlay = Overlay::Form(page_form()),
            "run_collectors" => self.run_default_collectors()?,
            "run_dns" => self.run_named_collector("dns")?,
            "run_rdap" => self.run_named_collector("rdap")?,
            "run_http" => self.run_named_collector("http")?,
            "run_tls" => self.run_named_collector("tls")?,
            "run_ct" => self.run_named_collector("ct")?,
            "run_ip" => self.run_named_collector("ip")?,
            "run_email" => self.run_named_collector("email")?,
            "run_username" => self.run_named_collector("username")?,
            "run_subdomain" => self.run_named_collector("subdomain")?,
            "ingest" => self.open_ingest_form(),
            "run_playbook" => self.open_playbook_form(FormKind::Playbook),
            "playbook_prompt" => self.open_playbook_form(FormKind::PlaybookPrompt),
            "watch_playbook" => self.open_playbook_form(FormKind::Watch),
            "ask_agent" => self.open_agent_prompt(),
            "generate_report" => self.generate_report("markdown")?,
            "add_finding" => self.overlay = Overlay::Form(finding_form()),
            "add_note" => self.overlay = Overlay::Form(note_form()),
            "open_entities" => self.switch_view(View::Entities),
            "open_sources" | "open_findings" | "open_evidence" => self.switch_view(View::Evidence),
            "open_timeline" => {
                self.time_lens = true;
                self.switch_view(View::Evidence);
            }
            "open_case" | "open_overview" | "open_pages" => self.switch_view(View::Overview),
            "open_review" => self.open_review_lane(),
            "open_audit" => self.switch_view(View::Audit),
            "set_marking" => self.open_marking_form(),
            "add_member" => self.open_member_form(),
            "toggle_nearby" => self.toggle_nearby(),
            "open_jobs" => self.switch_view(View::Jobs),
            "open_tools" => self.switch_view(View::Tools),
            "open_agent" => self.open_agent_history(),
            "open_settings" => self.switch_view(View::Settings),
            "theme" => self.open_theme_cycle(),
            "open_artifacts" => self.switch_view(View::Artifacts),
            "open_notes" => {
                self.inspector_tab = InspectorTab::Notes;
                self.switch_view(View::Overview);
                self.pane = Pane::Detail;
            }
            "open_object" => self.open_object_view(),
            "open_graph" => self.open_graph(),
            "search_around" => self.search_around_from_selection(),
            "merge_entities" => self.propose_merge(),
            "unresolve_entity" => self.propose_unresolve(),
            "resolution_suggestions" => self.open_suggestions(),
            "confirm_resolve" => self.confirm_resolve()?,
            "confirm_unresolve" => self.confirm_unresolve()?,
            "graph_confirm_cap" => self.confirm_graph_cap(),
            "doctor" => self.overlay = Overlay::Doctor,
            "help" => self.overlay = Overlay::Help,
            "cancel_job" => self.cancel_selected_job(),
            other if other.starts_with("run_tool:") => {
                self.spawn_tool(other.trim_start_matches("run_tool:"))?;
            }
            _ => self.status = format!("Unknown command {id}"),
        }
        Ok(())
    }

    fn cycle_form(&mut self, delta: i32) {
        if let Overlay::Form(form) = &mut self.overlay
            && ((form.kind == FormKind::NewPage && form.index == 1)
                || (form.kind == FormKind::Ingest && form.index == 0)
                || (form.kind == FormKind::Marking && form.index == 0)
                || (form.kind == FormKind::TeamMember && form.index == 1)
                || (matches!(
                    form.kind,
                    FormKind::Playbook | FormKind::PlaybookPrompt | FormKind::Watch
                ) && (form.index == 0 || form.index == 2)))
        {
            if matches!(
                form.kind,
                FormKind::Playbook | FormKind::PlaybookPrompt | FormKind::Watch
            ) {
                cycle_playbook_form(form, delta);
                return;
            }
            if form.kind == FormKind::Ingest {
                let kinds = crate::ingest::IngestFormat::ALL;
                let current = crate::ingest::IngestFormat::parse(&form.fields[0].value.value)
                    .unwrap_or(crate::ingest::IngestFormat::Stix);
                let index = kinds.iter().position(|kind| *kind == current).unwrap_or(0);
                let next = kinds[(index as i32 + delta).rem_euclid(kinds.len() as i32) as usize];
                form.fields[0].value.set(next.as_str());
                return;
            }
            if form.kind == FormKind::Marking {
                let kinds = crate::model::Marking::ALL;
                let current = crate::model::Marking::parse(&form.fields[0].value.value)
                    .unwrap_or(crate::model::Marking::Unmarked);
                let index = kinds.iter().position(|kind| *kind == current).unwrap_or(0);
                let next = kinds[(index as i32 + delta).rem_euclid(kinds.len() as i32) as usize];
                form.fields[0].value.set(next.as_str());
                return;
            }
            if form.kind == FormKind::TeamMember {
                let roles = [
                    crate::model::Role::Owner,
                    crate::model::Role::Analyst,
                    crate::model::Role::Reader,
                ];
                let current = crate::model::Role::parse(&form.fields[1].value.value)
                    .unwrap_or(crate::model::Role::Analyst);
                let index = roles.iter().position(|role| *role == current).unwrap_or(1);
                let next = roles[(index as i32 + delta).rem_euclid(roles.len() as i32) as usize];
                form.fields[1].value.set(next.as_str());
                return;
            }
            let kinds = PageKind::KNOWN;
            let current = PageKind::parse(&form.fields[1].value.value);
            let index = kinds.iter().position(|kind| kind == &current).unwrap_or(0);
            let next =
                kinds[(index as i32 + delta).rem_euclid(kinds.len() as i32) as usize].clone();
            form.fields[1].value.set(next.as_str());
            form.fields[2].hint = next.target_hint().to_string();
        }
    }

    fn insert_char(&mut self, ch: char) {
        match &mut self.overlay {
            Overlay::Palette { query, selected } | Overlay::Search { query, selected } => {
                query.insert(ch);
                *selected = 0;
            }
            Overlay::TableFilter { query } => {
                query.insert(ch);
            }
            Overlay::AgentPrompt { input } => input.insert(ch),
            Overlay::Form(form) => {
                if let Some(field) = form.current_mut() {
                    field.value.insert(ch);
                }
            }
            _ => {}
        }
        self.sync_table_filter();
    }

    fn backspace(&mut self) {
        self.with_input(Input::backspace);
    }
    fn delete_char(&mut self) {
        self.with_input(Input::delete);
    }
    fn input_left(&mut self) {
        self.with_input(Input::left);
    }
    fn input_right(&mut self) {
        self.with_input(Input::right);
    }
    fn input_home(&mut self) {
        self.with_input(Input::home);
    }
    fn input_end(&mut self) {
        self.with_input(Input::end);
    }

    fn with_input(&mut self, op: fn(&mut Input)) {
        match &mut self.overlay {
            Overlay::Palette { query, .. }
            | Overlay::Search { query, .. }
            | Overlay::TableFilter { query } => op(query),
            Overlay::AgentPrompt { input } => op(input),
            Overlay::Form(form) => {
                if let Some(field) = form.current_mut() {
                    op(&mut field.value);
                }
            }
            _ => {}
        }
        self.sync_table_filter();
    }

    fn sync_table_filter(&mut self) {
        if let Overlay::TableFilter { query } = &self.overlay {
            self.board_filter = query.value.clone();
            self.restore_board_selection();
        }
    }

    fn restore_board_selection(&mut self) {
        match self.view {
            View::Entities if !self.time_lens => {
                let keep = self.board_anchor_id.or(self.entity_id);
                let ids: Vec<_> = self
                    .listed_entities()
                    .iter()
                    .map(|entity| entity.id)
                    .collect();
                self.selected = keep
                    .and_then(|id| ids.iter().position(|candidate| *candidate == id))
                    .unwrap_or(0);
            }
            View::Evidence if !self.time_lens => {
                let keep = self.board_anchor_id.or(self.finding_id).or(self.source_id);
                let rows = self.listed_evidence();
                self.selected = keep
                    .and_then(|id| {
                        rows.iter().position(|row| match row {
                            EvidenceRow::Finding(row_id) | EvidenceRow::Source(row_id) => {
                                *row_id == id
                            }
                        })
                    })
                    .unwrap_or(0);
            }
            View::Jobs | View::Tools => {
                let len = self.board_len();
                if len == 0 || self.selected >= len {
                    self.selected = 0;
                }
            }
            _ => {}
        }
        self.sync_board_selection();
    }

    fn run_default_collectors(&mut self) -> Result<()> {
        let target = self.current_target().ok_or_else(|| {
            eyre!(
                "this page has no collectable target — create a Domain, IP, URL, or Repository page"
            )
        })?;
        let collectors = self.collectors.defaults_for(target.kind(), &self.tools);
        if collectors.is_empty() {
            bail_status(
                &mut self.status,
                "No collectors available for this page type.",
            );
            return Ok(());
        }
        for collector in collectors {
            self.spawn_collector(collector.id(), target.clone())?;
        }
        Ok(())
    }

    fn run_named_collector(&mut self, id: &str) -> Result<()> {
        let target = self
            .current_target()
            .ok_or_else(|| eyre!("no collectable target on this page"))?;
        self.spawn_collector(id, target)
    }

    fn spawn_collector(&mut self, id: &str, target: Target) -> Result<()> {
        let collector = self
            .collectors
            .find(id)
            .ok_or_else(|| eyre!("unknown collector {id}"))?;
        if !collector.accepts(target.kind()) {
            bail_status(
                &mut self.status,
                &format!("{} does not accept {}", collector.label(), target.kind()),
            );
            return Ok(());
        }
        let investigation_id = self.workspace.as_ref().map(Workspace::id);
        let page_id = self.page_id;
        let job_id = Uuid::new_v4();
        let mut record = JobRecord {
            id: job_id,
            investigation_id,
            seq: 0,
            kind: format!("collector:{id}"),
            label: collector.label().to_string(),
            description: format!("{} {}", collector.label(), target.value()),
            status: JobStatus::Queued,
            program: None,
            args: Vec::new(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            progress: None,
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id,
        };
        self.store.insert_job(&mut record)?;
        self.track_job(record);
        let net_error = self.net.as_ref().err().cloned();
        if collector.needs_network() && net_error.is_some() {
            return Err(eyre!(net_error.unwrap_or_default()));
        }
        let tools = Arc::clone(&self.tools);
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        let cache_dir = self.paths.cache_dir();
        let settings = CollectorSettings::from_config(&self.config);
        let net = self.net.as_ref().ok().cloned();
        self.jobs.spawn(job_id, move |job| async move {
            let net = net.ok_or_else(|| "network is not available".to_string())?;
            let context = CollectorContext {
                net,
                files,
                tools,
                cache_dir,
                settings,
                job,
            };
            collector
                .collect(&target, &context)
                .await
                .map(JobOutput::Collection)
        });
        self.status = format!("Queued {}", id);
        self.mark_dirty();
        Ok(())
    }

    fn ask_agent(&mut self, question: &str) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let page = self.current_page().cloned();
        let focus = ContextFocus {
            panel: format!("{:?}", self.view),
            reference: self.focus_ref(),
            label: self.focus_label(),
            kind: None,
            selected_text: None,
        };
        let collectors: Vec<String> = self
            .collectors
            .all()
            .iter()
            .map(|collector| collector.id().to_string())
            .collect();
        let thread_id = self.agent_thread.unwrap_or_else(Uuid::new_v4);
        self.agent_thread = Some(thread_id);
        let context = AgentContext::from_workspace(
            workspace,
            page.as_ref(),
            focus,
            question,
            &self.tools,
            &collectors,
            self.config.agent.history_turns,
            Some(thread_id),
        );
        let job_id = Uuid::new_v4();
        let mut run = AgentRun {
            id: Uuid::new_v4(),
            investigation_id: workspace.id(),
            seq: 0,
            thread_id,
            page_id: self.page_id,
            agent_name: self.agent.profile.name.clone(),
            program: self.agent.profile.program.clone(),
            args: Vec::new(),
            question: question.to_string(),
            response: String::new(),
            status: JobStatus::Queued,
            error: None,
            focus: self.focus_ref(),
            context_artifact_id: None,
            job_id: Some(job_id),
            model: None,
            cost_usd: None,
            created_at: now(),
            finished_at: None,
        };
        self.store.insert_agent_run(&mut run)?;
        self.agent_run_id = Some(run.id);
        let mut record = JobRecord {
            id: job_id,
            investigation_id: Some(workspace.id()),
            seq: 0,
            kind: "agent".into(),
            label: self.agent.profile.name.clone(),
            description: format!("Ask: {question}"),
            status: JobStatus::Queued,
            program: Some(self.agent.profile.program.clone()),
            args: Vec::new(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            progress: Some("Thinking...".into()),
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id: self.page_id,
        };
        self.store.insert_job(&mut record)?;
        self.track_job(record);
        self.streaming.insert(job_id, String::new());
        let profile = self.agent.profile.clone();
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        let max_context_bytes = self.config.agent.max_context_bytes;
        let run_id = run.id;
        self.jobs.spawn(job_id, move |job| async move {
            let request = AgentRequest {
                profile,
                context,
                files,
                max_context_bytes,
            };
            match run_agent(request, &job).await {
                Ok(outcome) => Ok(JobOutput::Agent(outcome)),
                Err(error) => Err(error),
            }
        });
        self.view = View::Review;
        self.status = format!(
            "Asking {}… Results land in Review.",
            self.agent.profile.name
        );
        let _ = run_id;
        Ok(())
    }

    fn focus_ref(&self) -> Option<String> {
        let workspace = self.workspace.as_ref()?;
        let id = match self.view {
            View::Entities | View::Object | View::Graph => self.entity_id,
            View::Sources | View::Evidence => self.source_id.or(self.finding_id),
            View::Findings => self.finding_id,
            _ => None,
        }?;
        workspace.ref_of(id).map(|reference| reference.to_string())
    }

    fn focus_label(&self) -> Option<String> {
        if !matches!(self.view, View::Entities | View::Object | View::Graph) {
            return None;
        }
        let workspace = self.workspace.as_ref()?;
        let id = self.entity_id?;
        workspace
            .entity(id)
            .map(|entity| entity.display_name().to_string())
    }

    fn generate_report(&mut self, format: &str) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        let blob = write_report(workspace, &files, format).map_err(|error| eyre!(error))?;
        self.store.add_artifact(
            workspace.id(),
            self.page_id,
            &crate::model::NewArtifact {
                kind: crate::model::ArtifactKind::Report,
                name: blob.relative_path.clone(),
                path: blob.relative_path.clone(),
                mime: Some(
                    if format == "json" {
                        "application/json"
                    } else {
                        "text/markdown"
                    }
                    .into(),
                ),
                size_bytes: blob.size_bytes as i64,
                sha256: Some(blob.sha256.clone()),
                fields: crate::model::Fields::new(),
                source: None,
            },
        )?;
        self.reload()?;
        self.status = format!("Wrote {}", blob.relative_path);
        self.overlay = Overlay::Text {
            title: "Report written".into(),
            body: files
                .read_text(&blob.relative_path, 200_000)
                .unwrap_or_default(),
            scroll: 0,
        };
        Ok(())
    }

    fn handle_job(&mut self, event: JobEvent) -> Result<()> {
        let mut dirty = true;
        let rebuild_lists = matches!(
            &event,
            JobEvent::Started { .. } | JobEvent::Command { .. } | JobEvent::Finished { .. }
        );
        match event {
            JobEvent::Started { job_id } => {
                if let Some(job) = self.job_records.get_mut(&job_id) {
                    job.status = JobStatus::Running;
                    job.started_at = Some(now());
                    self.store.update_job(job)?;
                }
            }
            JobEvent::Log {
                job_id,
                line,
                is_error,
            } => {
                if let Some(job) = self.job_records.get_mut(&job_id) {
                    let sink = if is_error {
                        &mut job.stderr
                    } else {
                        &mut job.log
                    };
                    sink.push_str(&line);
                    sink.push('\n');
                    if sink.len() > 32 * 1024 {
                        sink.drain(..sink.len() - 24 * 1024);
                    }
                    let _ = self.store.update_job(job);
                }
            }
            JobEvent::Progress { job_id, message } => {
                if let Some(job) = self.job_records.get_mut(&job_id) {
                    job.progress = Some(message.clone());
                    let _ = self.store.update_job(job);
                    if let Some((current, total)) =
                        crate::tui::motion::parse_known_progress(&message)
                    {
                        self.motion
                            .update_progress(&format!("prog:{job_id}"), current, total);
                    }
                }
                dirty = self.motion.allow_progress_redraw();
            }
            JobEvent::Command {
                job_id,
                program,
                args,
            } => {
                if let Some(job) = self.job_records.get_mut(&job_id) {
                    job.program = Some(program);
                    job.args = args;
                    let _ = self.store.update_job(job);
                }
            }
            JobEvent::AgentDelta { job_id, text } => {
                self.streaming.entry(job_id).or_default().push_str(&text);
            }
            JobEvent::AgentStatus { job_id, status } => {
                if let Some(job) = self.job_records.get_mut(&job_id) {
                    job.progress = Some(status);
                }
            }
            JobEvent::Finished { job_id, outcome } => {
                self.finish_job(job_id, outcome)?;
            }
        }
        if rebuild_lists {
            self.touch_workspace();
        }
        if dirty {
            self.mark_dirty();
        }
        Ok(())
    }

    fn finish_job(&mut self, job_id: Uuid, outcome: JobOutcome) -> Result<()> {
        self.jobs.finished(job_id);
        let mut result_text = None;
        let mut error_text = None;
        let mut status = JobStatus::Succeeded;
        let mut patched_in_place = false;
        match outcome {
            JobOutcome::Succeeded(output) => match *output {
                JobOutput::Collection(collected) => {
                    self.apply_collection(job_id, collected)?;
                    patched_in_place = true;
                    result_text = self
                        .job_records
                        .get(&job_id)
                        .and_then(|job| job.result.clone());
                }
                JobOutput::Agent(outcome) => {
                    self.apply_agent(job_id, outcome)?;
                }
                JobOutput::Report(report) => {
                    result_text = Some(format!("Wrote {}", report.relative_path));
                }
                JobOutput::Playbook(output) => {
                    self.apply_playbook_output(*output)?;
                    result_text = self
                        .job_records
                        .get(&job_id)
                        .and_then(|job| job.result.clone());
                }
            },
            JobOutcome::Failed(error) => {
                status = JobStatus::Failed;
                error_text = Some(error);
            }
            JobOutcome::Cancelled => {
                status = JobStatus::Cancelled;
                error_text = Some("cancelled".into());
            }
        }
        let progress_pair = self.job_records.get(&job_id).and_then(|job| {
            job.progress
                .as_deref()
                .and_then(crate::tui::motion::parse_known_progress)
        });
        if let Some(job) = self.job_records.get_mut(&job_id) {
            job.status = status;
            job.finished_at = Some(now());
            if let Some(result) = result_text {
                job.result = Some(result);
            }
            if let Some(error) = error_text.clone() {
                job.error = Some(error);
            }
            self.store.update_job(job)?;
        }
        if let Some((current, total)) = progress_pair {
            let key = format!("prog:{job_id}");
            if status == JobStatus::Succeeded {
                self.motion.complete_progress(&key, current, total);
            } else {
                self.motion.fail_progress(&key, current, total);
            }
        }
        if let Some(error) = error_text {
            self.status = error.clone();
            let label = self
                .job_records
                .get(&job_id)
                .map(|job| job.label.clone())
                .unwrap_or_else(|| "job".into());
            let kind = if error.contains("refused") {
                crate::tui::motion::ToastKind::Refused
            } else {
                crate::tui::motion::ToastKind::Err
            };
            self.motion.push_toast(kind, format!("{label} — {error}"));
        } else if status == JobStatus::Succeeded {
            let job = self.job_records.get(&job_id);
            let label = job.map(|job| job.label.as_str()).unwrap_or("job");
            let detail = job.and_then(|job| job.result.as_deref()).unwrap_or("done");
            self.motion.push_toast(
                crate::tui::motion::ToastKind::Ok,
                format!("{label} done · {detail}"),
            );
        }
        if patched_in_place {
            if let Some(job) = self.job_records.get(&job_id).cloned() {
                self.apply_workspace_delta(WorkspaceDelta {
                    jobs: vec![job],
                    ..WorkspaceDelta::default()
                })?;
            }
        } else {
            self.reload()?;
        }
        Ok(())
    }

    fn apply_collection(&mut self, job_id: Uuid, output: CollectionOutput) -> Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        let page_id = if self.job_records.contains_key(&job_id) {
            self.job_records.get(&job_id).and_then(|job| job.page_id)
        } else {
            self.page_id
        };
        let investigation_id = workspace.id();
        let summary = self.store.apply_batch(
            ApplyContext {
                investigation_id,
                page_id,
                job_id: Some(job_id),
            },
            &output.batch,
        )?;
        if let Some(job) = self.job_records.get_mut(&job_id) {
            job.result = Some(if output.batch.summary.is_empty() {
                summary.describe()
            } else {
                output.batch.summary.clone()
            });
        }
        let delta = self.store.load_apply_delta(investigation_id, &summary)?;
        self.apply_workspace_delta(delta)?;
        self.status = output.batch.summary;
        Ok(())
    }

    fn apply_agent(&mut self, job_id: Uuid, outcome: crate::agent::AgentOutcome) -> Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        let investigation_id = workspace.id();
        let Some(mut run) = self
            .store
            .load_investigation(investigation_id)?
            .agent_runs
            .into_iter()
            .find(|run| run.job_id == Some(job_id))
        else {
            self.status = "Agent result did not match an open run.".into();
            return Ok(());
        };
        let run_id = run.id;
        let run_page_id = run.page_id;
        {
            run.response = outcome.response.clone();
            run.status = if outcome.error.is_some() {
                JobStatus::Failed
            } else {
                JobStatus::Succeeded
            };
            run.error = outcome.error.clone();
            run.model = outcome.model.clone();
            run.cost_usd = outcome.cost_usd;
            run.args = outcome.args.clone();
            run.finished_at = Some(now());
            if !outcome.context_path.is_empty() {
                let artifact = NewArtifact {
                    kind: ArtifactKind::AgentContext,
                    name: outcome.context_path.clone(),
                    path: outcome.context_path.clone(),
                    mime: Some("text/markdown".into()),
                    size_bytes: 0,
                    sha256: None,
                    fields: crate::model::Fields::new(),
                    source: None,
                };
                if let Ok(artifact_id) =
                    self.store
                        .add_artifact(investigation_id, run_page_id, &artifact)
                {
                    run.context_artifact_id = Some(artifact_id);
                }
            }
            self.store.update_agent_run(&run)?;
            self.agent_run_id = Some(run_id);
        }
        let actions: Vec<(AgentAction, ProposalStatus, Option<String>)> = outcome
            .actions
            .into_iter()
            .map(|action| (action, ProposalStatus::Pending, None))
            .collect();
        if !actions.is_empty() {
            self.store
                .insert_proposals(investigation_id, run_id, actions)?;
            self.open_review_lane();
        }
        if let Some(stream) = self.streaming.get_mut(&job_id) {
            *stream = outcome.response;
        }
        if let Some(error) = outcome.error {
            self.status = error;
        } else {
            self.status = format!("{} replied", self.agent.profile.name);
        }
        Ok(())
    }

    fn decide_proposal(&mut self, status: ProposalStatus) -> Result<()> {
        if self.view == View::Review {
            return self.decide_review(status);
        }
        Ok(())
    }

    fn decide_review(&mut self, status: ProposalStatus) -> Result<()> {
        if status == ProposalStatus::Accepted && !self.review_armed {
            return Ok(());
        }
        let items = if status == ProposalStatus::Accepted {
            self.armed_review_items()
        } else {
            self.selected_review_item().into_iter().collect()
        };
        if items.is_empty() {
            self.review_armed = false;
            return Ok(());
        }
        if status == ProposalStatus::Accepted
            && items.iter().any(|item| item.has_missing_citation())
        {
            self.review_armed = false;
            self.status = "A citation points at a missing source. Acceptance is blocked.".into();
            return Ok(());
        }
        let accepted_ai = items.iter().any(|item| item.is_ai());
        for item in items {
            if let Some(proposal_id) = item.proposal_id {
                let Some(proposal) = self.workspace.as_ref().and_then(|workspace| {
                    workspace
                        .data
                        .proposals
                        .iter()
                        .find(|proposal| proposal.id == proposal_id)
                        .cloned()
                }) else {
                    continue;
                };
                self.apply_decided_proposal(&proposal, status)?;
            } else if let Some(merge) = item.merge {
                let entity_ids = merge.entity_ids.clone();
                if status == ProposalStatus::Accepted {
                    self.pending_resolve = Some(merge);
                    self.confirm_resolve()?;
                } else {
                    self.dismiss_suggestion(&entity_ids);
                    self.status = "Merge suggestion rejected.".into();
                }
                self.pending_merges
                    .retain(|pending| pending.entity_ids != entity_ids);
            }
        }
        self.review_armed = false;
        self.review_marks.clear();
        let remaining = self.listed_review_items().len();
        if remaining == 0 {
            self.selected = 0;
        } else if self.selected >= remaining {
            self.selected = remaining - 1;
        }
        if status == ProposalStatus::Accepted {
            self.motion.pulse_confirm();
            let glyph = if self.config.ui.ascii { "*" } else { "◆" };
            self.motion.push_toast(
                crate::tui::motion::ToastKind::Ok,
                format!("written · {glyph} AI"),
            );
            self.status = if accepted_ai {
                format!("Accepted. AI items keep {glyph} agent provenance.")
            } else {
                "Accepted. Analyst / playbook items keep their provenance.".into()
            };
        }
        Ok(())
    }

    fn dismiss_suggestion(&mut self, entity_ids: &[Uuid]) {
        let mut key = entity_ids.to_vec();
        key.sort();
        if !self
            .dismissed_suggestions
            .iter()
            .any(|existing| existing == &key)
        {
            self.dismissed_suggestions.push(key);
        }
    }

    fn apply_decided_proposal(
        &mut self,
        proposal: &Proposal,
        status: ProposalStatus,
    ) -> Result<()> {
        if proposal.status != ProposalStatus::Pending {
            return Ok(());
        }
        self.store.assert_can_mutate(proposal.investigation_id)?;
        if status == ProposalStatus::Accepted
            && let Err(error) = self.apply_proposal(proposal)
        {
            self.store.update_proposal(
                proposal.id,
                ProposalStatus::Invalid,
                Some(&error.to_string()),
            )?;
            self.status = error.to_string();
            self.reload()?;
            return Ok(());
        }
        self.store.update_proposal(proposal.id, status, None)?;
        self.reload()?;
        Ok(())
    }

    fn apply_proposal(&mut self, proposal: &Proposal) -> Result<()> {
        let action = &proposal.action;
        if action.is_reserved() {
            bail!(
                "{} is reserved and cannot be applied yet; AI only proposes implemented Actions",
                action.name()
            );
        }
        if matches!(
            action,
            AgentAction::ProposeResolve { .. } | AgentAction::ProposeUnresolve { .. }
        ) {
            self.apply_resolve_action(action)?;
            return Ok(());
        }
        if let AgentAction::RunPlaybook { playbook } = action {
            if !crate::playbook::is_bundled_id(playbook) {
                bail!(
                    "RUN_PLAYBOOK may only name a bundled playbook ({}), not a file path",
                    crate::playbook::bundled_ids().join(", ")
                );
            }
            self.spawn_named_playbook(playbook, false)?;
            return Ok(());
        }
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let investigation_id = workspace.id();
        match action {
            AgentAction::AddFinding {
                title,
                details,
                confidence,
                sources,
            } => {
                let source_ids = sources
                    .iter()
                    .filter_map(|reference| review::resolve_source_citation(workspace, reference))
                    .collect();
                self.store.create_finding(NewFinding {
                    investigation_id,
                    page_id: self.page_id,
                    title: title.clone(),
                    details: details.clone(),
                    confidence: confidence
                        .as_deref()
                        .and_then(Confidence::parse)
                        .unwrap_or(Confidence::Medium),
                    provenance: Provenance::Agent,
                    source_ids,
                })?;
            }
            AgentAction::AddNote { body } => {
                self.store
                    .create_note(investigation_id, self.page_id, body, Provenance::Agent)?;
            }
            AgentAction::CreatePage {
                title,
                page_type,
                target,
                description: _,
            } => {
                let kind = PageKind::parse(page_type.as_deref().unwrap_or("generic"));
                self.store.create_page(NewPage::new(
                    investigation_id,
                    title,
                    kind,
                    target.as_deref(),
                ))?;
            }
            AgentAction::LinkEntity {
                from,
                to,
                relation,
                sources,
            } => {
                let from_id = resolve_entity(workspace, from)
                    .ok_or_else(|| eyre!("unknown entity {from}"))?;
                let to_id =
                    resolve_entity(workspace, to).ok_or_else(|| eyre!("unknown entity {to}"))?;
                let source_ids: Vec<Uuid> = sources
                    .iter()
                    .filter_map(|reference| review::resolve_source_citation(workspace, reference))
                    .collect();
                self.store.link_entities(
                    investigation_id,
                    from_id,
                    relation,
                    to_id,
                    Provenance::Agent,
                    Confidence::Medium,
                    &source_ids,
                )?;
            }
            AgentAction::RunCollector { collector, target } => {
                let parsed = Target::guess(target).map_err(|error| eyre!(error))?;
                self.spawn_collector(collector, parsed)?;
            }
            AgentAction::GenerateReport { format } => {
                self.generate_report(format.as_deref().unwrap_or("markdown"))?;
            }
            AgentAction::ProposeResolve { .. } | AgentAction::ProposeUnresolve { .. } => {
                unreachable!("resolution actions apply before this match")
            }
            AgentAction::RunPlaybook { .. } => {
                unreachable!("RUN_PLAYBOOK applies before this match")
            }
            AgentAction::AssertFindingTemplate { title, details } => {
                let provenance = self
                    .workspace
                    .as_ref()
                    .and_then(|workspace| {
                        workspace
                            .data
                            .agent_runs
                            .iter()
                            .find(|run| run.id == proposal.agent_run_id)
                    })
                    .map(|run| {
                        if run.agent_name == crate::storage::PLAYBOOK_AGENT_NAME {
                            Provenance::Analyst
                        } else {
                            Provenance::Agent
                        }
                    })
                    .unwrap_or(Provenance::Agent);
                self.store.create_finding(NewFinding {
                    investigation_id,
                    page_id: self.page_id,
                    title: title.clone(),
                    details: details.clone(),
                    confidence: Confidence::Medium,
                    provenance,
                    source_ids: Vec::new(),
                })?;
            }
        }
        Ok(())
    }

    fn open_object_view(&mut self) {
        if self.workspace.is_none() {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        let listed_entity_id = self
            .listed_entities()
            .get(self.selected)
            .map(|entity| entity.id);
        let current_page = self.workspace.as_ref().and_then(|workspace| {
            self.page_id
                .and_then(|id| workspace.page(id).map(|page| (page.id, page.entity_id)))
                .or_else(|| {
                    workspace
                        .data
                        .pages
                        .get(self.selected)
                        .map(|page| (page.id, page.entity_id))
                })
        });
        match self.view {
            View::Entities => {
                self.entity_id = listed_entity_id;
            }
            View::Graph => {
                self.entity_id = self
                    .visible_graph()
                    .nodes
                    .get(self.graph.selected)
                    .map(|node| node.id);
                if self.entity_id.is_none() {
                    self.status = "No node selected.".into();
                    return;
                }
            }
            View::Object => {}
            View::Pages
            | View::Overview
            | View::Evidence
            | View::Review
            | View::Welcome
            | View::Sources
            | View::Findings
            | View::Timeline
            | View::Audit
            | View::Notes
            | View::Agent
            | View::Jobs
            | View::Tools
            | View::Settings
            | View::Artifacts => {
                if let Some((page_id, entity_id)) = current_page {
                    self.page_id = Some(page_id);
                    self.entity_id = entity_id;
                } else {
                    self.entity_id = None;
                }
            }
        }
        if let Some(id) = self.entity_id {
            self.entity_id = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(id).map(|_| workspace.canonical_id(id)));
        }
        let has_entity = self
            .entity_id
            .and_then(|id| {
                self.workspace
                    .as_ref()
                    .and_then(|workspace| workspace.entity(id))
            })
            .is_some();
        let has_page = self
            .page_id
            .and_then(|id| {
                self.workspace
                    .as_ref()
                    .and_then(|workspace| workspace.page(id))
            })
            .is_some();
        if !has_entity && !has_page {
            self.status = "Select an entity or page to open Object View.".into();
            return;
        }
        if self.view != View::Object {
            self.object_return_view = Some(self.view);
        }
        self.view = View::Object;
        self.scroll = 0;
        self.overlay = Overlay::None;
        self.select_nav_for_object_view();
        self.status = "Object View. Esc returns.".into();
        let _ = self.save_session();
    }

    fn select_nav_for_object_view(&mut self) {
        let target = if self.entity_id.is_some() {
            View::Entities
        } else {
            View::Overview
        };
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == target)
        {
            self.nav_index = index;
        }
    }

    fn close_object_view(&mut self) {
        let return_view = self.object_return_view.take().unwrap_or(View::Entities);
        self.view = Self::canonicalize_view(if return_view == View::Object {
            View::Entities
        } else {
            return_view
        });
        self.scroll = 0;
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == self.view)
        {
            self.nav_index = index;
        }
        if self.view == View::Timeline {
            self.restore_timeline_selection();
        } else if self.view == View::Entities
            && let Some(id) = self.entity_id
        {
            let listed = self.listed_entities();
            if let Some(index) = listed.iter().position(|entity| entity.id == id) {
                self.selected = index;
            }
        }
        let _ = self.save_session();
    }

    pub fn current_object_view(&self) -> Option<crate::model::ObjectView> {
        let workspace = self.workspace.as_ref()?;
        if let Some(entity) = self.entity_id.and_then(|id| workspace.entity(id)) {
            return Some(crate::model::ObjectView::for_entity(workspace, entity));
        }
        let page = self
            .page_id
            .and_then(|id| workspace.page(id))
            .or_else(|| workspace.data.pages.get(self.selected))?;
        Some(crate::model::ObjectView::for_page(workspace, page))
    }

    pub fn search_hits(&self, query: &str) -> Vec<SearchHit> {
        let query = query.trim().to_ascii_lowercase();
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        if query.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        let push = |hits: &mut Vec<SearchHit>,
                    hay: &str,
                    label: String,
                    kind: &str,
                    id: Uuid,
                    view: View| {
            if hay.to_ascii_lowercase().contains(&query) {
                hits.push(SearchHit {
                    label,
                    kind: kind.into(),
                    id,
                    view,
                });
            }
        };
        for page in &workspace.data.pages {
            push(
                &mut hits,
                &format!(
                    "{} {} {}",
                    page.title,
                    page.target.as_deref().unwrap_or(""),
                    page.notes
                ),
                format!("{} {}", page.short_ref(), page.title),
                "page",
                page.id,
                View::Overview,
            );
        }
        for entity in &workspace.data.entities {
            push(
                &mut hits,
                &format!("{} {}", entity.value, entity.label),
                format!("{} {}", entity.short_ref(), entity.display_name()),
                "entity",
                entity.id,
                View::Entities,
            );
        }
        for finding in &workspace.data.findings {
            push(
                &mut hits,
                &format!("{} {}", finding.title, finding.details),
                format!("{} {}", finding.short_ref(), finding.title),
                "finding",
                finding.id,
                View::Evidence,
            );
        }
        for source in &workspace.data.sources {
            push(
                &mut hits,
                &format!("{} {}", source.title, source.content),
                format!("{} {}", source.short_ref(), source.title),
                "source",
                source.id,
                View::Evidence,
            );
        }
        for artifact in &workspace.data.artifacts {
            push(
                &mut hits,
                &format!("{} {}", artifact.name, artifact.path),
                format!("{} {}", artifact.short_ref(), artifact.name),
                "artifact",
                artifact.id,
                View::Artifacts,
            );
        }
        for run in &workspace.data.agent_runs {
            push(
                &mut hits,
                &format!("{} {}", run.question, run.response),
                format!("{} {}", run.short_ref(), run.question),
                "agent",
                run.id,
                View::Review,
            );
        }
        hits.truncate(80);
        hits
    }

    pub fn job_status_line(&self) -> String {
        let active: Vec<&JobRecord> = self
            .job_records
            .values()
            .filter(|job| job.status.is_active())
            .collect();
        if active.is_empty() {
            return format!("{} jobs", self.job_records.len());
        }
        let parts: Vec<String> = active
            .iter()
            .map(|job| format!("{} {}", job.label, job.status.symbol()))
            .collect();
        format!("{} jobs | {}", active.len(), parts.join(" | "))
    }

    /// Findings then sources. The Evidence board is this list unless the time lens is on.
    pub fn listed_evidence(&self) -> Vec<EvidenceRow> {
        if let Some(rows) = self.cached_board_rows(|cache| cache.evidence.clone()) {
            return self.revalidate_listed_evidence(rows);
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        rows.extend(
            workspace
                .data
                .findings
                .iter()
                .map(|finding| EvidenceRow::Finding(finding.id)),
        );
        rows.extend(
            workspace
                .data
                .sources
                .iter()
                .map(|source| EvidenceRow::Source(source.id)),
        );
        let filter = self.parsed_board_filter();
        rows.retain(|row| filter::evidence_row_matches(&filter, *row, workspace));
        if let Some(sort) = self.effective_sort() {
            rows.sort_by(|left, right| filter::cmp_evidence(*left, *right, workspace, sort));
        }
        self.store_board_rows(|cache| cache.evidence = Some(rows.clone()));
        rows
    }

    pub fn unfiltered_evidence_count(&self) -> usize {
        let Some(workspace) = self.workspace.as_ref() else {
            return 0;
        };
        workspace.data.findings.len() + workspace.data.sources.len()
    }

    /// Jobs in the order the Jobs pane lists them: persisted rows, then live jobs not reloaded yet.
    pub fn listed_job_ids(&self) -> Vec<Uuid> {
        if let Some(ids) = self.cached_board_rows(|cache| cache.jobs.clone()) {
            return self.revalidate_listed_jobs(ids);
        }
        let mut ids = Vec::new();
        if let Some(workspace) = self.workspace.as_ref() {
            ids.extend(workspace.data.jobs.iter().map(|job| job.id));
        }
        let mut live: Vec<&JobRecord> = self
            .job_records
            .values()
            .filter(|job| !ids.contains(&job.id))
            .collect();
        live.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        ids.extend(live.iter().map(|job| job.id));
        let filter = self.parsed_board_filter();
        ids.retain(|id| {
            let live = self.job_records.get(id);
            let stored = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.job(*id));
            let label = live
                .map(|job| job.label.as_str())
                .or_else(|| stored.map(|job| job.label.as_str()))
                .unwrap_or("");
            let status = live
                .map(|job| job.status.as_str())
                .or_else(|| stored.map(|job| job.status.as_str()))
                .unwrap_or("");
            let progress = live
                .and_then(|job| job.progress.as_deref())
                .or_else(|| stored.and_then(|job| job.progress.as_deref()))
                .unwrap_or("");
            filter.matches_text(&format!("{label} {status} {progress}"))
        });
        self.store_board_rows(|cache| cache.jobs = Some(ids.clone()));
        ids
    }

    pub fn unfiltered_job_count(&self) -> usize {
        let mut ids = Vec::new();
        if let Some(workspace) = self.workspace.as_ref() {
            ids.extend(workspace.data.jobs.iter().map(|job| job.id));
        }
        let extra: Vec<_> = self
            .job_records
            .keys()
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        ids.extend(extra);
        ids.len()
    }

    pub fn listed_tool_indices(&self) -> Vec<usize> {
        if let Some(indices) = self.cached_board_rows(|cache| cache.tools.clone()) {
            return self.revalidate_listed_tools(indices);
        }
        let filter = self.parsed_board_filter();
        let indices: Vec<usize> = self
            .tools
            .tools()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let hay = format!(
                    "{} {} {}",
                    entry.definition.name,
                    entry.definition.activity.as_str(),
                    entry.problem.as_deref().unwrap_or("")
                );
                filter.matches_text(&hay)
            })
            .map(|(index, _)| index)
            .collect();
        self.store_board_rows(|cache| cache.tools = Some(indices.clone()));
        indices
    }

    fn board_cache_key(&self) -> String {
        let workspace = self.workspace.as_ref();
        let tools = self
            .tools
            .tools()
            .iter()
            .map(|entry| entry.definition.name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{:?}|{}|{}|{}|{}|{}|{:?}|{}|{:?}",
            self.view,
            self.active_board_filter(),
            self.timeline.nearby,
            self.time_lens,
            workspace.map(Workspace::id).unwrap_or(Uuid::nil()),
            self.workspace_rev,
            self.timeline.nearby_seed_id,
            tools,
            self.effective_sort()
        )
    }

    pub(crate) fn revalidate_listed_entity_ids(&self, ids: Vec<Uuid>) -> Vec<Uuid> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let universe: HashSet<Uuid> = if self.timeline.nearby && self.view == View::Entities {
            self.listed_nearby()
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        } else {
            workspace
                .visible_entities()
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for id in ids {
            let canonical = workspace.canonical_id(id);
            if universe.contains(&canonical) && seen.insert(canonical) {
                out.push(canonical);
            }
        }
        out
    }

    fn revalidate_listed_evidence(&self, rows: Vec<EvidenceRow>) -> Vec<EvidenceRow> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        rows.into_iter()
            .filter(|row| {
                let id = match row {
                    EvidenceRow::Finding(id) if workspace.finding(*id).is_some() => *id,
                    EvidenceRow::Source(id) if workspace.source(*id).is_some() => *id,
                    _ => return false,
                };
                seen.insert(id)
            })
            .collect()
    }

    fn revalidate_listed_jobs(&self, ids: Vec<Uuid>) -> Vec<Uuid> {
        let mut seen = HashSet::new();
        ids.into_iter()
            .filter(|id| {
                let exists = self.job_records.contains_key(id)
                    || self
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| workspace.job(*id).is_some());
                exists && seen.insert(*id)
            })
            .collect()
    }

    fn revalidate_listed_tools(&self, indices: Vec<usize>) -> Vec<usize> {
        let n = self.tools.tools().len();
        let mut seen = HashSet::new();
        indices
            .into_iter()
            .filter(|index| *index < n && seen.insert(*index))
            .collect()
    }

    fn cached_board_rows<T>(&self, pick: impl Fn(&BoardListCache) -> Option<T>) -> Option<T> {
        let key = self.board_cache_key();
        let cache = self.board_list_cache.borrow();
        let cache = cache.as_ref()?;
        if cache.key != key {
            return None;
        }
        pick(cache)
    }

    fn store_board_rows(&self, write: impl FnOnce(&mut BoardListCache)) {
        let key = self.board_cache_key();
        let mut cache = self.board_list_cache.borrow_mut();
        match cache.as_mut() {
            Some(existing) if existing.key == key => write(existing),
            _ => {
                let mut fresh = BoardListCache {
                    key,
                    entities: None,
                    evidence: None,
                    jobs: None,
                    tools: None,
                };
                write(&mut fresh);
                *cache = Some(fresh);
            }
        }
    }

    pub fn cancel_selected_job(&mut self) {
        let Some(job_id) = self.listed_job_ids().get(self.selected).copied() else {
            return;
        };
        self.jobs.cancel(job_id);
        if let Some(job) = self.job_records.get(&job_id) {
            self.status = format!("Cancelling {}", job.label);
        } else if let Some(job) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.job(job_id))
        {
            self.status = format!("Cancelling {}", job.label);
        }
    }

    pub fn is_editing(&self) -> bool {
        matches!(
            self.overlay,
            Overlay::Palette { .. }
                | Overlay::Search { .. }
                | Overlay::TableFilter { .. }
                | Overlay::AgentPrompt { .. }
                | Overlay::Form(_)
        )
    }

    pub fn has_active_jobs(&self) -> bool {
        self.jobs.running_count() > 0 || self.job_records.values().any(|job| job.status.is_active())
    }

    pub fn doctor_checks(&self) -> Vec<crate::doctor::Check> {
        crate::doctor::run(&self.config, &self.paths, Some(&self.store), &self.tools)
    }

    fn spawn_tool(&mut self, name: &str) -> Result<()> {
        let Some(entry) = self.tools.find(name).cloned() else {
            self.set_error(format!("unknown tool {name}"));
            return Ok(());
        };
        if !entry.is_available() {
            self.set_error(format!(
                "{} is not available: {}",
                entry.definition.name,
                entry.problem.as_deref().unwrap_or("not found on PATH")
            ));
            return Ok(());
        }
        if entry.definition.activity == Activity::Active {
            self.set_error(format!(
                "{} is an active probe. os-int only runs passive observation and diagnostic tools.",
                entry.definition.name
            ));
            return Ok(());
        }
        self.spawn_tool_inner(entry)
    }

    fn spawn_tool_inner(&mut self, entry: ToolEntry) -> Result<()> {
        let target = self
            .current_target()
            .ok_or_else(|| eyre!("this page has no collectable target"))?;
        if !entry.definition.accepts(target.kind()) {
            bail_status(
                &mut self.status,
                &format!(
                    "{} does not accept {}",
                    entry.definition.name,
                    target.kind()
                ),
            );
            return Ok(());
        }
        let program = entry
            .path
            .clone()
            .ok_or_else(|| eyre!("{} is not installed", entry.definition.name))?;
        let investigation_id = self.workspace.as_ref().map(Workspace::id);
        let page_id = self.page_id;
        let job_id = Uuid::new_v4();
        let mut record = JobRecord {
            id: job_id,
            investigation_id,
            seq: 0,
            kind: format!("tool:{}", entry.definition.name),
            label: entry.definition.name.clone(),
            description: format!("{} {}", entry.definition.name, target.value()),
            status: JobStatus::Queued,
            program: Some(entry.definition.program.clone()),
            args: Vec::new(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            progress: None,
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id,
        };
        self.store.insert_job(&mut record)?;
        self.track_job(record);
        let definition = entry.definition.clone();
        self.jobs.spawn(job_id, move |job| async move {
            run_tool(&definition, program, &target, &job)
                .await
                .map(JobOutput::Collection)
        });
        self.status = format!("Queued {}", entry.definition.name);
        self.view = View::Jobs;
        Ok(())
    }

    fn open_ingest_form(&mut self) {
        if self.workspace.is_none() {
            self.status = "Open an investigation before ingesting a file.".into();
            return;
        }
        self.overlay = Overlay::Form(ingest_form());
    }

    fn submit_ingest_form(&mut self, form: FormState) -> Result<()> {
        let values: Vec<String> = form
            .fields
            .iter()
            .map(|field| field.value.value.trim().to_string())
            .collect();
        let result = match form.kind {
            FormKind::Ingest => {
                let path = values.get(1).map(String::as_str).unwrap_or("");
                if path.is_empty() {
                    self.status = "Ingest needs a file path.".into();
                    return Ok(());
                }
                self.apply_ingest(&values)
            }
            FormKind::Playbook | FormKind::Watch => self.submit_playbook_form(&form, &values),
            FormKind::PlaybookPrompt => self.submit_playbook_prompt(&values),
            _ => Ok(()),
        };
        match result {
            Ok(()) => {
                if !matches!(
                    self.overlay,
                    Overlay::AgentPrompt { .. } | Overlay::Text { .. }
                ) {
                    self.overlay = Overlay::None;
                }
                Ok(())
            }
            Err(error) => {
                self.set_error(error.to_string());
                Ok(())
            }
        }
    }

    fn open_playbook_form(&mut self, kind: FormKind) {
        if self.workspace.is_none() {
            self.status = "Open an investigation before running a playbook.".into();
            return;
        }
        let target = self
            .current_target()
            .map(|target| target.value())
            .unwrap_or_default();
        self.overlay = Overlay::Form(playbook_form(kind, &target));
    }

    fn submit_playbook_form(&mut self, form: &FormState, values: &[String]) -> Result<()> {
        let spec = values.first().map(String::as_str).unwrap_or("").trim();
        if spec.is_empty() {
            bail!("choose a bundled playbook or a YAML file");
        }
        let target_raw = values.get(1).map(String::as_str).unwrap_or("").trim();
        let mode = values.get(2).map(String::as_str).unwrap_or("run");
        let dry_run = mode == "dry-run";
        let automation = form.kind == FormKind::Watch;
        self.launch_playbook(spec, target_raw, dry_run, automation)
    }

    fn submit_playbook_prompt(&mut self, values: &[String]) -> Result<()> {
        let spec = values.first().map(String::as_str).unwrap_or("domain-infra");
        let playbook =
            crate::playbook::load_playbook(spec).map_err(|error| eyre!(error.to_string()))?;
        let seed = values
            .get(1)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| self.current_target().map(|target| target.value()))
            .unwrap_or_else(|| "example.com".into());
        let question = crate::playbook::bundled_prompt(&playbook, &seed);
        self.overlay = Overlay::AgentPrompt {
            input: Input::new(&question),
        };
        self.status = "Playbook prompt seeded. Edit if needed, then Ctrl+S to send.".into();
        Ok(())
    }

    fn launch_playbook(
        &mut self,
        spec: &str,
        target_raw: &str,
        dry_run: bool,
        automation: bool,
    ) -> Result<()> {
        let selected = self.selected;
        let entity_id = self.entity_id;
        let playbook =
            crate::playbook::load_playbook(spec).map_err(|error| eyre!(error.to_string()))?;
        let seed = self.playbook_seed(&playbook, target_raw)?;
        let tools = Arc::clone(&self.tools);
        let job = crate::jobs::JobContext::detached(false);
        if dry_run {
            let mut plan =
                crate::playbook::plan_playbook(&playbook, &seed, &tools, automation, &job)
                    .map_err(|error| eyre!(error.to_string()))?;
            if let Some(workspace) = &self.workspace {
                crate::playbook::bind_resolve_plan(&mut plan, &workspace.data, &playbook);
            }
            self.selected = selected;
            self.entity_id = entity_id;
            self.overlay = Overlay::Text {
                title: if automation {
                    "Automation dry-run".into()
                } else {
                    "Playbook dry-run".into()
                },
                body: plan.describe(),
                scroll: 0,
            };
            self.status = "Dry-run: nothing was applied.".into();
            return Ok(());
        }
        self.spawn_playbook_job(playbook, seed, automation)?;
        self.selected = selected;
        if let Some(id) = entity_id {
            self.entity_id = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(id).map(|_| workspace.canonical_id(id)));
        }
        Ok(())
    }

    fn playbook_seed(&self, playbook: &crate::model::Playbook, target_raw: &str) -> Result<Target> {
        if !target_raw.is_empty() {
            return Target::parse(playbook.seed.kind, target_raw).map_err(|error| eyre!(error));
        }
        if let Some(target) = self.current_target()
            && target.kind() == playbook.seed.kind
        {
            return Ok(target);
        }
        bail!(
            "playbook {} needs a {} seed; enter a target",
            playbook.id,
            playbook.seed.kind
        )
    }

    fn spawn_named_playbook(&mut self, spec: &str, automation: bool) -> Result<()> {
        if self
            .job_records
            .values()
            .any(|job| job.kind.starts_with("playbook:") && job.status.is_active())
        {
            bail!("a playbook is already running; nested playbooks are refused");
        }
        let playbook =
            crate::playbook::load_playbook(spec).map_err(|error| eyre!(error.to_string()))?;
        let seed = self.playbook_seed(&playbook, "")?;
        self.spawn_playbook_job(playbook, seed, automation)
    }

    fn spawn_playbook_job(
        &mut self,
        playbook: crate::model::Playbook,
        seed: Target,
        automation: bool,
    ) -> Result<()> {
        if self
            .job_records
            .values()
            .any(|job| job.kind.starts_with("playbook:") && job.status.is_active())
        {
            bail!("a playbook is already running; nested playbooks are refused");
        }
        let investigation_id = self.workspace.as_ref().map(Workspace::id);
        let page_id = self.page_id;
        let job_id = Uuid::new_v4();
        let mut record = JobRecord {
            id: job_id,
            investigation_id,
            seq: 0,
            kind: format!("playbook:{}", playbook.id),
            label: playbook.name.clone(),
            description: format!("{} {}", playbook.id, seed.value()),
            status: JobStatus::Queued,
            program: None,
            args: Vec::new(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            progress: None,
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id,
        };
        self.store.insert_job(&mut record)?;
        self.track_job(record);
        let net_error = self.net.as_ref().err().cloned();
        if net_error.is_some() {
            return Err(eyre!(net_error.unwrap_or_default()));
        }
        let tools = Arc::clone(&self.tools);
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        let cache_dir = self.paths.cache_dir();
        let settings = CollectorSettings::from_config(&self.config);
        let net = self.net.as_ref().ok().cloned();
        let collectors = self.collectors.clone();
        let previous_digest = investigation_id.and_then(|id| {
            self.store
                .find_automation(id, &playbook.id, &seed.value())
                .ok()
                .flatten()
                .and_then(|row| row.last_digest)
        });
        let options = crate::playbook::ExecuteOptions {
            dry_run: false,
            automation,
            previous_digest,
            interval_seconds: None,
        };
        let playbook_id = playbook.id.clone();
        self.jobs.spawn(job_id, move |job| async move {
            let net = net.ok_or_else(|| "network is not available".to_string())?;
            let context = CollectorContext {
                net,
                files,
                tools: Arc::clone(&tools),
                cache_dir,
                settings,
                job,
            };
            crate::playbook::gather(&playbook, &seed, &collectors, &context, options)
                .await
                .map(|output| JobOutput::Playbook(Box::new(output)))
                .map_err(|error| error.message)
        });
        self.status = format!("Queued playbook {playbook_id}");
        self.view = View::Jobs;
        self.mark_dirty();
        Ok(())
    }

    fn apply_playbook_output(&mut self, output: crate::model::PlaybookOutput) -> Result<()> {
        let Some(investigation_id) = self.workspace.as_ref().map(Workspace::id) else {
            return Ok(());
        };
        let page_id = self.page_id;
        if output.dry_run {
            self.overlay = Overlay::Text {
                title: "Playbook dry-run".into(),
                body: output.plan.describe(),
                scroll: 0,
            };
            self.status = "Dry-run: nothing was applied.".into();
            return Ok(());
        }
        let playbook = output.playbook.clone();
        let seed =
            Target::parse(playbook.seed.kind, &output.seed_value).map_err(|error| eyre!(error))?;
        let committed = crate::playbook::commit(
            &mut self.store,
            &playbook,
            &seed,
            ApplyContext {
                investigation_id,
                page_id,
                job_id: None,
            },
            output.clone(),
            crate::playbook::ExecuteOptions {
                dry_run: false,
                automation: output.automation,
                previous_digest: output.previous_digest.clone(),
                interval_seconds: None,
            },
        )
        .map_err(|error| eyre!(error.to_string()))?;
        if let Some(job) = self
            .job_records
            .values_mut()
            .find(|job| job.kind == format!("playbook:{}", playbook.id) && job.status.is_active())
        {
            job.result = Some(format!("{} pending proposal(s)", committed.proposals.len()));
        }
        self.status = format!(
            "Playbook {} finished. {} pending proposal(s).",
            playbook.id,
            committed.proposals.len()
        );
        if !committed.proposals.is_empty()
            && let Some(run_id) = self
                .store
                .load_investigation(investigation_id)
                .ok()
                .and_then(|data| {
                    data.agent_runs
                        .iter()
                        .rev()
                        .find(|run| run.agent_name == crate::storage::PLAYBOOK_AGENT_NAME)
                        .map(|run| run.id)
                })
        {
            self.open_review_lane();
            let _ = run_id;
        }
        Ok(())
    }

    fn apply_ingest(&mut self, values: &[String]) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let format = crate::ingest::IngestFormat::parse(
            values.first().map(String::as_str).unwrap_or("stix"),
        )
        .ok_or_else(|| eyre!("unknown ingest format"))?;
        let path = values.get(1).map(String::as_str).unwrap_or("").trim();
        let selected = self.selected;
        let entity_id = self.entity_id;
        let max_bytes = self.config.collectors.max_ingest_bytes;
        let bytes = crate::ingest::read_ingest_file(std::path::Path::new(path), max_bytes)
            .map_err(|error| eyre!(error))?;
        let mut mapping = values
            .get(3)
            .filter(|value| !value.is_empty())
            .map(|spec| crate::ingest::ObservableMapping::parse(spec))
            .transpose()
            .map_err(|error| eyre!(error))?
            .unwrap_or_default();
        if let Some(kind) = values.get(2).filter(|value| !value.is_empty()) {
            mapping.default_kind = Some(crate::model::EntityKind::parse(kind));
        }
        let origin = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("ingest")
            .to_string();
        let report = crate::ingest::ingest(
            format,
            &bytes,
            &crate::ingest::IngestOptions { mapping, origin },
        )
        .map_err(|error| eyre!(error))?;
        let summary = self.store.apply_batch(
            ApplyContext {
                investigation_id: workspace.id(),
                page_id: self.page_id,
                job_id: None,
            },
            &report.batch,
        )?;
        self.reload()?;
        self.selected = selected;
        if let Some(id) = entity_id {
            self.entity_id = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(id).map(|_| workspace.canonical_id(id)));
        }
        self.status = if report.batch.summary.is_empty() {
            summary.describe()
        } else {
            report.batch.summary
        };
        Ok(())
    }
}

fn field(label: &str, value: &str, hint: &str) -> Field {
    Field {
        label: label.into(),
        value: Input::new(value),
        hint: hint.into(),
    }
}

fn page_form() -> FormState {
    FormState::new(
        FormKind::NewPage,
        vec![
            field("Title", "", "Acme Corp"),
            field("Type", "domain", "Tab to cycle"),
            field("Target", "", PageKind::Domain.target_hint()),
        ],
    )
}

fn finding_form() -> FormState {
    FormState::new(
        FormKind::NewFinding,
        vec![
            field("Finding", "", "one sentence"),
            field("Details", "", "optional"),
            field("Confidence", "medium", "low/medium/high"),
            field("Sources", "", "S-1, S-2"),
        ],
    )
}

fn note_form() -> FormState {
    FormState::new(FormKind::NewNote, vec![field("Note", "", "Markdown")])
}

impl App {
    fn open_marking_form(&mut self) {
        let current = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.investigation().marking.as_str().to_string())
            .unwrap_or_else(|| "unmarked".into());
        self.overlay = Overlay::Form(FormState::new(
            FormKind::Marking,
            vec![field(
                "Marking",
                &current,
                "Left/Right: unmarked TLP:CLEAR GREEN AMBER AMBER+STRICT RED",
            )],
        ));
    }

    fn open_member_form(&mut self) {
        self.overlay = Overlay::Form(FormState::new(
            FormKind::TeamMember,
            vec![
                field("User id", "", "matches [identity] user_id"),
                field("Role", "analyst", "Left/Right: owner analyst reader"),
            ],
        ));
    }
}

fn ingest_form() -> FormState {
    FormState::new(
        FormKind::Ingest,
        vec![
            field("Format", "stix", "Left/Right: stix misp csv json batch"),
            field("Path", "", "path to the file"),
            field("Kind", "", "optional default kind for CSV"),
            field("Mapping", "", "optional kind=col,value=col"),
        ],
    )
}

fn playbook_form(kind: FormKind, target: &str) -> FormState {
    let id = crate::playbook::bundled_ids()
        .first()
        .copied()
        .unwrap_or("domain-infra");
    let mode = match kind {
        FormKind::Watch => "dry-run",
        FormKind::PlaybookPrompt => "send",
        _ => "run",
    };
    let mode_hint = match kind {
        FormKind::Watch => "Left/Right: dry-run once",
        FormKind::PlaybookPrompt => "Enter seeds the agent question",
        _ => "Left/Right: run dry-run",
    };
    FormState::new(
        kind,
        vec![
            field(
                "Playbook",
                id,
                "Left/Right: bundled ids, or type a YAML path",
            ),
            field("Target", target, "seed value (example.com)"),
            field("Mode", mode, mode_hint),
        ],
    )
}

fn cycle_playbook_form(form: &mut FormState, delta: i32) {
    let ids = crate::playbook::bundled_ids();
    if form.index == 0 && !ids.is_empty() {
        let current = form.fields[0].value.value.trim();
        let index = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = ids[(index as i32 + delta).rem_euclid(ids.len() as i32) as usize];
        form.fields[0].value.set(next);
        return;
    }
    if form.index == 2 && form.kind != FormKind::PlaybookPrompt {
        let modes: &[&str] = if form.kind == FormKind::Watch {
            &["dry-run", "once"]
        } else {
            &["run", "dry-run"]
        };
        let current = form.fields[2].value.value.trim();
        let index = modes.iter().position(|mode| *mode == current).unwrap_or(0);
        let next = modes[(index as i32 + delta).rem_euclid(modes.len() as i32) as usize];
        form.fields[2].value.set(next);
    }
}

fn resolve_entity(workspace: &Workspace, raw: &str) -> Option<Uuid> {
    if let Ok(reference) = raw.parse() {
        return workspace.resolve_ref(reference);
    }
    workspace.entity_by_any_value(raw).map(|entity| entity.id)
}

fn bail_status(status: &mut String, message: &str) {
    *status = message.to_string();
}

fn add_scroll(scroll: u16, delta: i32) -> u16 {
    (i32::from(scroll) + delta).max(0) as u16
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn test_app() -> (tempfile::TempDir, App) {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let app = App::new(Config::default(), paths, store);
        (directory, app)
    }

    #[test]
    fn first_run_is_welcome() {
        let (_directory, mut app) = test_app();
        app.open_last_or_welcome().expect("open");
        assert_eq!(app.view, View::Welcome);
    }

    #[test]
    fn creating_an_investigation_and_domain_page_round_trips() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Example Corp");
        }
        app.dispatch(Action::Submit).expect("create");
        assert!(app.workspace.is_some());
        app.dispatch(Action::Command("new_page"))
            .expect("page form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("example.com");
            form.fields[1].value.set("domain");
            form.fields[2].value.set("Example.COM");
        }
        app.dispatch(Action::Submit).expect("page");
        let workspace = app.workspace.as_ref().expect("workspace");
        assert_eq!(
            workspace.data.pages[0].target.as_deref(),
            Some("example.com")
        );
        assert_eq!(workspace.data.entities[0].value, "example.com");
    }

    #[test]
    fn command_palette_filters() {
        let hits = filtered("dns");
        assert!(hits.iter().any(|command| command.id == "run_dns"));
    }

    #[test]
    fn investigation_survives_restart_with_findings_notes_and_report() {
        let (directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Example Corp");
        }
        app.dispatch(Action::Submit).expect("create");
        app.dispatch(Action::Command("new_page")).expect("page");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("example.com");
            form.fields[1].value.set("domain");
            form.fields[2].value.set("example.com");
        }
        app.dispatch(Action::Submit).expect("page");
        app.dispatch(Action::Command("add_finding"))
            .expect("finding");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Apex domain is registered");
            form.fields[1]
                .value
                .set("Placeholder until collectors run.");
        }
        app.dispatch(Action::Submit).expect("save finding");
        app.dispatch(Action::Command("add_note")).expect("note");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Follow the nameservers next.");
        }
        app.dispatch(Action::Submit).expect("save note");
        app.dispatch(Action::Command("generate_report"))
            .expect("report");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.dispatch(Action::Quit).expect("save session");
        assert!(app.should_quit);
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        let store = Store::open(&paths.database()).expect("reopen");
        let mut restarted = App::new(Config::default(), paths, store);
        restarted.open_last_or_welcome().expect("restore");
        assert_eq!(
            restarted.workspace.as_ref().unwrap().investigation().name,
            "Example Corp"
        );
        assert_eq!(restarted.view, View::Entities);
        let workspace = restarted.workspace.as_ref().expect("workspace");
        assert_eq!(
            workspace.data.pages[0].target.as_deref(),
            Some("example.com")
        );
        assert_eq!(
            workspace.data.findings[0].title,
            "Apex domain is registered"
        );
        assert!(
            workspace.data.pages[0]
                .notes
                .contains("Follow the nameservers")
                || workspace
                    .data
                    .notes
                    .iter()
                    .any(|note| note.body.contains("Follow the nameservers"))
        );
        assert!(
            workspace
                .data
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::Report)
        );
    }

    fn create_named_investigation(app: &mut App, name: &str) {
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set(name);
        }
        app.dispatch(Action::Submit).expect("create");
    }

    fn create_page(app: &mut App, title: &str, kind: &str, target: &str) {
        app.dispatch(Action::Command("new_page"))
            .expect("page form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set(title);
            form.fields[1].value.set(kind);
            form.fields[2].value.set(target);
        }
        app.dispatch(Action::Submit).expect("page");
    }

    fn sample_job(label: &str, status: JobStatus, page_id: Option<Uuid>) -> JobRecord {
        JobRecord {
            id: Uuid::new_v4(),
            investigation_id: None,
            seq: 0,
            kind: "collector:dns".into(),
            label: label.into(),
            description: format!("{label} job"),
            status,
            program: None,
            args: Vec::new(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            progress: None,
            log: String::new(),
            stderr: String::new(),
            result: None,
            error: None,
            page_id,
        }
    }

    fn sample_run(
        investigation_id: Uuid,
        job_id: Uuid,
        thread_id: Uuid,
        question: &str,
    ) -> AgentRun {
        AgentRun {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 0,
            thread_id,
            page_id: None,
            agent_name: "Claude Code".into(),
            program: "claude".into(),
            args: Vec::new(),
            question: question.into(),
            response: String::new(),
            status: JobStatus::Running,
            error: None,
            focus: None,
            context_artifact_id: None,
            job_id: Some(job_id),
            model: None,
            cost_usd: None,
            created_at: now(),
            finished_at: None,
        }
    }

    #[test]
    fn welcome_keys_move_the_welcome_menu() {
        let (_directory, mut app) = test_app();
        app.open_last_or_welcome().expect("welcome");
        app.dispatch(Action::Move(1)).expect("down");
        assert_eq!(app.selected, 1);
        app.dispatch(Action::Select).expect("open");
        assert!(matches!(
            app.overlay,
            Overlay::Form(ref form) if form.kind == FormKind::OpenInvestigation
        ));
    }

    #[test]
    fn collection_stays_on_the_page_that_queued_it() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        create_page(&mut app, "first", "domain", "first.example");
        create_page(&mut app, "second", "domain", "second.example");
        let pages = app
            .workspace
            .as_ref()
            .expect("workspace")
            .data
            .pages
            .clone();
        let queued_page = pages[0].id;
        let current_page = pages[1].id;
        app.page_id = Some(current_page);
        let job_id = Uuid::new_v4();
        let mut job = sample_job("DNS", JobStatus::Running, Some(queued_page));
        job.id = job_id;
        job.investigation_id = Some(app.workspace.as_ref().expect("workspace").id());
        app.job_records.insert(job_id, job);
        let mut batch = crate::model::EvidenceBatch::new();
        batch.add_finding(
            "queued on the first page",
            "collected while another page was open",
            Confidence::High,
            None,
        );
        app.apply_collection(job_id, CollectionOutput::new(batch))
            .expect("applies");
        app.reload().expect("reload");
        let finding = &app.workspace.as_ref().expect("workspace").data.findings[0];
        assert_eq!(finding.page_id, Some(queued_page));
    }

    #[test]
    fn agent_result_updates_the_run_that_owns_the_job() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let finished_job = Uuid::new_v4();
        let selected_job = Uuid::new_v4();
        let mut finished = sample_run(
            investigation_id,
            finished_job,
            Uuid::new_v4(),
            "What is public?",
        );
        let mut selected = sample_run(
            investigation_id,
            selected_job,
            Uuid::new_v4(),
            "A later question",
        );
        app.store.insert_agent_run(&mut finished).expect("run");
        app.store.insert_agent_run(&mut selected).expect("run");
        app.agent_run_id = Some(selected.id);
        app.apply_agent(
            finished_job,
            crate::agent::AgentOutcome {
                response: "The domain is registered.".into(),
                actions: Vec::new(),
                model: None,
                cost_usd: None,
                error: None,
                context_path: String::new(),
                program: "claude".into(),
                args: Vec::new(),
            },
        )
        .expect("applies");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        let finished_run = data
            .agent_runs
            .iter()
            .find(|run| run.id == finished.id)
            .expect("finished run");
        let selected_run = data
            .agent_runs
            .iter()
            .find(|run| run.id == selected.id)
            .expect("selected run");
        assert_eq!(finished_run.response, "The domain is registered.");
        assert_eq!(finished_run.status, JobStatus::Succeeded);
        assert!(selected_run.response.is_empty());
        assert_eq!(selected_run.status, JobStatus::Running);
        assert_eq!(app.agent_run_id, Some(finished.id));
    }

    #[test]
    fn accepting_a_proposal_applies_the_highlighted_pending_action() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Notes?");
        app.store.insert_agent_run(&mut run).expect("run");
        let proposals = app
            .store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![
                    (
                        AgentAction::AddNote {
                            body: "already saved".into(),
                        },
                        ProposalStatus::Accepted,
                        None,
                    ),
                    (
                        AgentAction::AddNote {
                            body: "still pending".into(),
                        },
                        ProposalStatus::Pending,
                        None,
                    ),
                ],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        let pending = data
            .proposals
            .iter()
            .find(|proposal| proposal.id == proposals[1].id)
            .expect("pending");
        assert_eq!(pending.status, ProposalStatus::Accepted);
        assert!(
            data.notes
                .iter()
                .any(|note| note.body.contains("still pending"))
        );
        assert!(
            !data
                .notes
                .iter()
                .any(|note| note.body.contains("already saved"))
        );
    }

    #[test]
    fn proposal_arming_is_required_before_accept_via_keys() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Arm Case");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddNote {
                        body: "armed accept".into(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        assert!(app.review_armed);
        app.dispatch(Action::Disarm).expect("disarm");
        assert!(!app.review_armed);
        app.dispatch(Action::Arm).expect("re-arm");
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert!(
            data.notes
                .iter()
                .any(|note| note.body.contains("armed accept"))
        );
    }

    #[test]
    fn unarmed_accept_does_not_apply_a_proposal() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Unarmed");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddNote {
                        body: "must stay pending".into(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Accept).expect("refused");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.proposals[0].status, ProposalStatus::Pending);
        assert!(
            data.notes
                .iter()
                .all(|note| !note.body.contains("must stay"))
        );
        assert!(!app.review_armed);
    }

    #[test]
    fn review_enter_a_and_y_alone_do_not_write() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Review Safety");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "must stay pending".into(),
                        details: String::new(),
                        confidence: None,
                        sources: Vec::new(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Select).expect("enter");
        app.dispatch(Action::AskAgent).ok();
        app.dispatch(Action::Accept).expect("y alone");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.proposals[0].status, ProposalStatus::Pending);
        assert!(data.findings.is_empty());
    }

    #[test]
    fn review_accept_keeps_agent_provenance() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Keep Diamond");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "phishing host".into(),
                        details: "impersonation".into(),
                        confidence: Some("medium".into()),
                        sources: Vec::new(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.findings[0].provenance, Provenance::Agent);
        assert_eq!(data.proposals[0].status, ProposalStatus::Accepted);
    }

    #[test]
    fn hallway_create_target_accept_ai_export_without_help() {
        let (directory, mut app) = test_app();
        create_named_investigation(&mut app, "Hallway");
        create_page(&mut app, "acme-login.com", "domain", "acme-login.com");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page_id = app.workspace.as_ref().expect("workspace").data.pages[0].id;
        let job_id = Uuid::new_v4();
        let mut job = sample_job("DNS", JobStatus::Succeeded, Some(page_id));
        job.id = job_id;
        job.investigation_id = Some(investigation_id);
        app.job_records.insert(job_id, job);
        let mut batch = crate::model::EvidenceBatch::new();
        let source = crate::model::NewSource::collected(
            crate::model::SourceKind::Dns,
            "dns",
            "DNS records for acme-login.com",
        );
        let key = batch.add_source(source);
        batch.entity(
            crate::model::EntityKind::Domain,
            "acme-login.com",
            Some(key),
        );
        app.apply_collection(job_id, CollectionOutput::new(batch))
            .expect("collect");
        app.reload().expect("reload");
        let collected = app
            .workspace
            .as_ref()
            .expect("workspace")
            .data
            .sources
            .iter()
            .find(|source| source.title.contains("DNS records"))
            .expect("collected source");
        let source_ref = collected.short_ref().to_string();
        assert_eq!(source_ref, "S-1");
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Phish?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "acme-login.com is phishing infra".into(),
                        details: "SAN mimics acme.com".into(),
                        confidence: Some("medium".into()),
                        sources: vec![source_ref],
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        assert!(!matches!(app.overlay, Overlay::Help));
        app.view = View::Overview;
        let home = app.case_home();
        assert!(!home.targets.is_empty());
        app.open_review_lane();
        let item = app.selected_review_item().expect("review item");
        assert!(!item.has_missing_citation());
        let arm = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('A'),
            crossterm::event::KeyModifiers::SHIFT,
        );
        let action = crate::tui::keymap::map_key(
            arm,
            &app.overlay,
            app.view,
            app.is_editing(),
            app.review_armed,
            app.board_zoomed,
        )
        .expect("Shift+A");
        assert!(matches!(action, Action::Arm));
        app.dispatch(action).expect("arm");
        assert!(app.review_armed);
        let yes = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('y'),
            crossterm::event::KeyModifiers::NONE,
        );
        let action = crate::tui::keymap::map_key(
            yes,
            &app.overlay,
            app.view,
            app.is_editing(),
            app.review_armed,
            app.board_zoomed,
        )
        .expect("y");
        assert!(matches!(action, Action::Accept));
        app.dispatch(action).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.findings[0].provenance, Provenance::Agent);
        assert_eq!(data.findings[0].title, "acme-login.com is phishing infra");
        assert_eq!(data.findings[0].source_ids.len(), 1);
        assert_eq!(data.proposals[0].status, ProposalStatus::Accepted);
        let dest = directory.path().join("export");
        let files = crate::files::InvestigationFiles::new(&app.paths, investigation_id);
        crate::exchange::export_investigation(
            &mut app.store,
            investigation_id,
            files.root(),
            &dest,
        )
        .expect("export");
        let archive = dest.join("investigation.osint.json");
        let body = std::fs::read_to_string(&archive).expect("export file");
        assert!(body.contains("Hallway"), "{body}");
        assert!(body.contains("acme-login.com is phishing infra"), "{body}");
        assert!(body.contains("DNS records for acme-login.com"), "{body}");
        assert!(body.contains("\"agent\""), "{body}");
        assert!(!matches!(app.overlay, Overlay::Help));
    }

    #[test]
    fn review_tick_never_accepts_or_disarms() {
        let (_directory, mut app) = test_app();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        create_named_investigation(&mut app, "Tick Cannot Write");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "must stay pending across ticks".into(),
                        details: String::new(),
                        confidence: None,
                        sources: Vec::new(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        assert!(app.review_armed);
        for _ in 0..200 {
            app.dispatch(Action::Tick).expect("tick");
            assert!(
                app.review_armed,
                "the 60 ms tick must not disarm or auto-confirm"
            );
            assert_ne!(
                app.motion.confirm_phase(),
                crate::tui::motion::ConfirmPhase::Idle
            );
        }
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.proposals[0].status, ProposalStatus::Pending);
        assert!(data.findings.is_empty());
    }

    #[test]
    fn missing_citation_blocks_review_accept() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Missing Cite");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "blocked".into(),
                        details: String::new(),
                        confidence: None,
                        sources: vec!["S-99".into()],
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        let item = app.selected_review_item().expect("item");
        assert!(item.has_missing_citation());
        app.dispatch(Action::Arm).expect("arm");
        assert!(!app.review_armed);
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.proposals[0].status, ProposalStatus::Pending);
        assert!(data.findings.is_empty());
    }

    #[test]
    fn reader_accept_leaves_finding_and_proposal_unchanged() {
        let (_directory, mut app) = test_app();
        app.store.set_identity(crate::model::Identity {
            user_id: "alice".into(),
            machine_name: "desk-a".into(),
        });
        create_named_investigation(&mut app, "Shared Case");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_member(investigation_id, "carol", crate::model::Role::Reader)
            .expect("carol");
        let mut run = sample_run(investigation_id, Uuid::new_v4(), Uuid::new_v4(), "Find?");
        app.store.insert_agent_run(&mut run).expect("run");
        let proposals = app
            .store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::AddFinding {
                        title: "reader must not persist this".into(),
                        details: String::new(),
                        confidence: None,
                        sources: Vec::new(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.store.set_identity(crate::model::Identity {
            user_id: "carol".into(),
            machine_name: "desk-c".into(),
        });
        app.reload().expect("reload as reader");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        let error = app.dispatch(Action::Accept).expect_err("reader accept");
        assert!(
            error.to_string().contains("reader"),
            "unexpected error: {error}"
        );
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert!(data.findings.is_empty());
        let pending = data
            .proposals
            .iter()
            .find(|proposal| proposal.id == proposals[0].id)
            .expect("proposal");
        assert_eq!(pending.status, ProposalStatus::Pending);
    }

    #[test]
    fn reader_write_commands_fail_closed() {
        let (_directory, mut app) = test_app();
        app.store.set_identity(crate::model::Identity {
            user_id: "alice".into(),
            machine_name: "desk-a".into(),
        });
        create_named_investigation(&mut app, "Shared Case");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_member(investigation_id, "carol", crate::model::Role::Reader)
            .expect("carol");
        app.store.set_identity(crate::model::Identity {
            user_id: "carol".into(),
            machine_name: "desk-c".into(),
        });
        app.reload().expect("reload");
        let error = app
            .dispatch(Action::Command("add_finding"))
            .expect_err("finding");
        assert!(error.to_string().contains("reader"), "{error}");
        let error = app
            .dispatch(Action::Command("set_marking"))
            .expect_err("marking");
        assert!(error.to_string().contains("owner"), "{error}");
        let error = app
            .dispatch(Action::Command("run_collectors"))
            .expect_err("collect");
        assert!(error.to_string().contains("reader"), "{error}");
        let error = app.dispatch(Action::NewItem).expect_err("keyboard n");
        assert!(error.to_string().contains("reader"), "{error}");
        let error = app.dispatch(Action::Refresh).expect_err("keyboard r");
        assert!(error.to_string().contains("reader"), "{error}");
        let error = app.dispatch(Action::AskAgent).expect_err("keyboard a");
        assert!(error.to_string().contains("reader"), "{error}");
    }

    #[test]
    fn search_open_restores_the_agent_thread() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let older_thread = Uuid::new_v4();
        let newer_thread = Uuid::new_v4();
        let mut older = sample_run(investigation_id, Uuid::new_v4(), older_thread, "Older");
        let mut newer = sample_run(investigation_id, Uuid::new_v4(), newer_thread, "Newer");
        app.store.insert_agent_run(&mut older).expect("run");
        app.store.insert_agent_run(&mut newer).expect("run");
        app.reload().expect("reload");
        app.agent_thread = Some(newer_thread);
        app.open_hit(&SearchHit {
            label: "Older".into(),
            kind: "agent".into(),
            id: older.id,
            view: View::Review,
        });
        assert_eq!(app.view, View::Review);
        assert_eq!(app.agent_run_id, Some(older.id));
        assert_eq!(app.agent_thread, Some(older_thread));
        assert_eq!(
            app.selected_review_item().and_then(|item| item.run_id),
            Some(older.id)
        );
    }

    #[test]
    fn cancel_targets_the_listed_job_not_the_first_active_one() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut stored = sample_job("Stored", JobStatus::Queued, None);
        stored.investigation_id = Some(investigation_id);
        app.store.insert_job(&mut stored).expect("job");
        app.reload().expect("reload");
        app.job_records.insert(stored.id, stored.clone());
        let mut live = sample_job("Live", JobStatus::Running, None);
        live.investigation_id = Some(investigation_id);
        live.created_at = now();
        app.job_records.insert(live.id, live.clone());
        app.view = View::Jobs;
        app.selected = 1;
        assert_eq!(app.listed_job_ids()[1], live.id);
        app.cancel_selected_job();
        assert_eq!(app.status, "Cancelling Live");
    }

    #[test]
    fn agent_focus_follows_the_current_view() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        create_page(&mut app, "example.com", "domain", "example.com");
        let entity_id = app.workspace.as_ref().expect("workspace").data.entities[0].id;
        app.entity_id = Some(entity_id);
        app.view = View::Overview;
        assert!(app.focus_ref().is_none());
        assert!(app.focus_label().is_none());
        app.view = View::Entities;
        assert_eq!(app.focus_ref().as_deref(), Some("E-1"));
        assert_eq!(app.focus_label().as_deref(), Some("example.com"));
    }

    #[test]
    fn enter_on_entities_opens_object_view_with_property_group_and_links() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        create_page(&mut app, "example.com", "domain", "example.com");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page_id = app.workspace.as_ref().expect("workspace").data.pages[0].id;
        let mut batch = crate::model::EvidenceBatch::new();
        let source = crate::model::NewSource::collected(
            crate::model::SourceKind::Dns,
            "dns",
            "DNS records for example.com",
        );
        let key = batch.add_source(source);
        let domain = crate::model::EntityKey::new(crate::model::EntityKind::Domain, "example.com");
        let ip = crate::model::EntityKey::new(crate::model::EntityKind::Ip, "192.0.2.10");
        batch.relate(&domain, "resolves_to", &ip, Some(key));
        batch.annotate(&domain, "registrar", "Example Registrar");
        batch.relate(&domain, "spotted_near", &ip, Some(key));
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: Some(page_id),
                    job_id: None,
                },
                &batch,
            )
            .expect("applies");
        app.reload().expect("reload");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::Select).expect("object view");
        assert_eq!(app.view, View::Object);
        let view = app.current_object_view().expect("object view");
        assert!(
            view.groups
                .iter()
                .any(|group| group.id == "identity" && !group.properties.is_empty())
        );
        assert!(view.links.iter().any(|group| group.catalogued));
        assert!(
            view.links
                .iter()
                .any(|group| group.label == crate::model::ontology::OTHER_LINK_GROUP)
        );
        app.dispatch(Action::Back).expect("back");
        assert_eq!(app.view, View::Entities);
    }

    #[test]
    fn open_object_view_uses_highlighted_entity_not_leftover() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        create_page(&mut app, "alpha.example", "domain", "alpha.example");
        create_page(&mut app, "beta.example", "domain", "beta.example");
        app.reload().expect("reload");
        let (entity_a, entity_b) = {
            let workspace = app.workspace.as_ref().expect("workspace");
            (workspace.data.entities[0].id, workspace.data.entities[1].id)
        };
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::OpenObjectView).expect("open A");
        assert_eq!(app.entity_id, Some(entity_a));
        assert_eq!(app.current_object_view().expect("A").title, "alpha.example");
        app.dispatch(Action::Back).expect("return");
        app.selected = 1;
        app.dispatch(Action::OpenObjectView).expect("open B");
        assert_eq!(app.entity_id, Some(entity_b));
        assert_eq!(app.current_object_view().expect("B").title, "beta.example");
    }

    #[test]
    fn restoring_object_view_selects_entities_nav() {
        let (directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        create_page(&mut app, "example.com", "domain", "example.com");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.dispatch(Action::OpenObjectView).expect("object");
        assert_eq!(app.view, View::Object);
        app.dispatch(Action::Quit).expect("save session");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        let store = Store::open(&paths.database()).expect("reopen");
        let mut restarted = App::new(Config::default(), paths, store);
        restarted.open_last_or_welcome().expect("restore");
        assert_eq!(restarted.view, View::Object);
        assert_eq!(restarted.nav_items()[restarted.nav_index].1, View::Entities);
    }

    fn seed_infra(app: &mut App) {
        create_named_investigation(app, "Example Corp");
        create_page(app, "example.com", "domain", "example.com");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page_id = app.workspace.as_ref().expect("workspace").data.pages[0].id;
        let mut batch = crate::model::EvidenceBatch::new();
        let source = crate::model::NewSource::collected(
            crate::model::SourceKind::Dns,
            "dns",
            "DNS records for example.com",
        );
        let key = batch.add_source(source);
        let domain = crate::model::EntityKey::new(crate::model::EntityKind::Domain, "example.com");
        let ip = crate::model::EntityKey::new(crate::model::EntityKind::Ip, "192.0.2.10");
        let nameserver =
            crate::model::EntityKey::new(crate::model::EntityKind::Hostname, "ns1.example.net");
        let asn = crate::model::EntityKey::new(crate::model::EntityKind::Asn, "AS64496");
        let ns_ip = crate::model::EntityKey::new(crate::model::EntityKind::Ip, "192.0.2.53");
        batch.relate(&domain, "resolves_to", &ip, Some(key));
        batch.relate(&domain, "uses_nameserver", &nameserver, Some(key));
        batch.relate(&ip, "announced_by", &asn, Some(key));
        batch.relate(&nameserver, "resolves_to", &ns_ip, Some(key));
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: Some(page_id),
                    job_id: None,
                },
                &batch,
            )
            .expect("applies");
        app.reload().expect("reload");
    }

    #[test]
    fn graph_opens_connected_infra_and_search_around_reaches_two_hops() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        assert!(app.graph.result.nodes.len() >= 3);
        assert!(!app.visible_graph().is_empty());
        let before = app.graph.result.nodes.len();
        app.dispatch(Action::SearchAround).expect("search around");
        assert_eq!(app.view, View::Graph);
        assert!(app.graph.result.nodes.len() >= before);
        let workspace = app.workspace.as_ref().expect("workspace");
        let asn = workspace
            .data
            .entities
            .iter()
            .find(|entity| entity.kind == crate::model::EntityKind::Asn)
            .expect("asn");
        assert!(
            app.graph.result.node_ids().any(|id| id == asn.id),
            "Search Around 2 hops should reach the ASN"
        );
        app.dispatch(Action::Back).expect("back");
        assert_eq!(app.view, View::Entities);
    }

    #[test]
    fn closing_graph_restores_entities_selection_to_focused_entity() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        let (ip_id, ip_index) = {
            let workspace = app.workspace.as_ref().expect("workspace");
            workspace
                .data
                .entities
                .iter()
                .enumerate()
                .find(|(_, entity)| entity.kind == crate::model::EntityKind::Ip)
                .map(|(index, entity)| (entity.id, index))
                .expect("ip")
        };
        assert_ne!(ip_index, 0);
        app.selected = ip_index;
        app.sync_board_selection();
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        app.dispatch(Action::Back).expect("close graph");
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, ip_index);
        assert_eq!(app.entity_id, Some(ip_id));
        app.dispatch(Action::Select).expect("object view");
        assert_eq!(app.view, View::Object);
        assert_eq!(
            app.current_object_view().expect("object").title,
            "192.0.2.10"
        );
    }

    #[test]
    fn empty_visible_graph_does_not_open_stale_object_view() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenGraph).expect("graph");
        let ip_id = app
            .workspace
            .as_ref()
            .expect("workspace")
            .data
            .entities
            .iter()
            .find(|entity| entity.kind == crate::model::EntityKind::Ip)
            .map(|entity| entity.id)
            .expect("ip");
        let chips = app.graph_facet_chips();
        app.graph.facet_index = chips
            .iter()
            .position(|chip| chip.key == "ip")
            .expect("ip facet");
        app.dispatch(Action::GraphApplyFacet).expect("facet");
        assert_eq!(app.entity_id, Some(ip_id));
        assert_eq!(app.pane, Pane::Main);
        app.dispatch(Action::GraphHops(-1)).expect("zero hops");
        assert_eq!(app.graph.hops, 0);
        assert!(app.graph.filters.kind.is_none());
        assert_ne!(app.entity_id, Some(ip_id));
        app.graph.filters.kind = Some("person".into());
        app.refresh_graph_selection();
        assert!(app.visible_graph().is_empty());
        assert!(app.entity_id.is_none());
        app.dispatch(Action::Select).expect("enter");
        assert_eq!(app.view, View::Graph);
        assert_eq!(app.status, "No node selected.");
        app.dispatch(Action::OpenObjectView).expect("v");
        assert_eq!(app.view, View::Graph);
        assert_eq!(app.status, "No node selected.");
    }

    #[test]
    fn graph_facet_filter_changes_visible_set() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenGraph).expect("graph");
        app.dispatch(Action::SearchAround).expect("expand");
        let unfiltered = app.visible_graph().nodes.len();
        assert!(unfiltered > 1);
        let chips = app.graph_facet_chips();
        let ip_index = chips
            .iter()
            .position(|chip| chip.key == "ip")
            .expect("ip facet");
        app.graph.facet_index = ip_index;
        app.dispatch(Action::GraphApplyFacet).expect("facet");
        let filtered = app.visible_graph();
        assert!(filtered.nodes.len() < unfiltered);
        assert!(filtered.nodes.iter().all(|node| {
            app.workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(node.id))
                .is_some_and(|entity| entity.kind == crate::model::EntityKind::Ip)
        }));
        app.dispatch(Action::GraphClearFilters).expect("clear");
        assert_eq!(app.visible_graph().nodes.len(), unfiltered);
    }

    #[test]
    fn graph_caps_require_confirm_and_then_render() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.graph.node_cap = 2;
        app.dispatch(Action::OpenGraph).expect("graph");
        assert!(
            matches!(app.overlay, Overlay::Confirm { ref command, .. } if command == "graph_confirm_cap")
        );
        assert!(app.graph.pending.is_some());
        app.dispatch(Action::Select).expect("confirm");
        assert_eq!(app.view, View::Graph);
        assert_eq!(app.graph.result.nodes.len(), 2);
        assert!(app.graph.result.truncated);
        assert!(app.graph.pending.is_none());
    }

    #[test]
    fn escaping_cap_dialog_clears_pending() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.graph.node_cap = 2;
        app.dispatch(Action::OpenGraph).expect("graph");
        assert!(app.graph.pending.is_some());
        app.dispatch(Action::Cancel).expect("esc");
        assert!(app.graph.pending.is_none());
        assert_eq!(app.overlay, Overlay::None);
        assert_ne!(app.view, View::Graph);
    }

    #[test]
    fn restoring_graph_keeps_search_around_hops_and_seeds() {
        let (directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenGraph).expect("graph");
        app.dispatch(Action::SearchAround).expect("two hops");
        assert_eq!(app.graph.hops, 2);
        let asn_id = app
            .workspace
            .as_ref()
            .expect("workspace")
            .data
            .entities
            .iter()
            .find(|entity| entity.kind == crate::model::EntityKind::Asn)
            .map(|entity| entity.id)
            .expect("asn");
        assert!(app.graph.result.node_ids().any(|id| id == asn_id));
        let seeds = app.graph.seed_ids.clone();
        app.dispatch(Action::Quit).expect("save session");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        let store = Store::open(&paths.database()).expect("reopen");
        let mut restarted = App::new(Config::default(), paths, store);
        restarted.open_last_or_welcome().expect("restore");
        assert_eq!(restarted.view, View::Graph);
        assert_eq!(restarted.graph.hops, 2);
        assert_eq!(restarted.graph.seed_ids, seeds);
        assert!(restarted.graph.result.node_ids().any(|id| id == asn_id));
    }

    #[test]
    fn changing_hops_requeries_from_original_seed() {
        let (_directory, mut app) = test_app();
        seed_infra(&mut app);
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.graph.hops, 1);
        let domain_id = app.graph.seed_ids[0];
        let (ip_id, asn_id) = {
            let workspace = app.workspace.as_ref().expect("workspace");
            let ip = workspace
                .data
                .entities
                .iter()
                .find(|entity| entity.kind == crate::model::EntityKind::Ip)
                .map(|entity| entity.id)
                .expect("ip");
            let asn = workspace
                .data
                .entities
                .iter()
                .find(|entity| entity.kind == crate::model::EntityKind::Asn)
                .map(|entity| entity.id)
                .expect("asn");
            (ip, asn)
        };
        assert!(!app.graph.result.node_ids().any(|id| id == asn_id));
        let ip_index = app
            .visible_graph()
            .nodes
            .iter()
            .position(|node| node.id == ip_id)
            .expect("ip on graph");
        app.graph.selected = ip_index;
        app.dispatch(Action::GraphHops(1)).expect("more hops");
        assert_eq!(app.graph.hops, app.graph.result.hops_used);
        assert_eq!(app.graph.hops, 2);
        assert_eq!(app.graph.seed_ids, vec![domain_id]);
        assert!(app.graph.result.node_ids().any(|id| id == asn_id));
        app.dispatch(Action::GraphHops(-1)).expect("fewer hops");
        assert_eq!(app.graph.hops, 1);
        assert_eq!(app.graph.seed_ids, vec![domain_id]);
        assert!(!app.graph.result.node_ids().any(|id| id == asn_id));
    }

    #[test]
    fn graph_empty_investigation_does_not_panic() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Empty");
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        assert!(app.visible_graph().is_empty());
        app.dispatch(Action::SearchAround).expect("around");
        app.dispatch(Action::GraphApplyFacet).expect("facet");
        app.dispatch(Action::GraphClearFilters).expect("clear");
        app.dispatch(Action::Move(1)).expect("move");
        app.dispatch(Action::Select).expect("select");
    }

    #[test]
    fn palette_graph_commands_exist() {
        assert!(
            filtered("graph")
                .iter()
                .any(|command| command.id == "open_graph")
        );
        assert!(
            filtered("search around")
                .iter()
                .any(|command| command.id == "search_around")
        );
    }

    #[test]
    fn run_playbook_file_path_is_refused_and_records_the_local_actor() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Example Corp");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut run = sample_run(
            investigation_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Playbook?",
        );
        app.store.insert_agent_run(&mut run).expect("run");
        let proposals = app
            .store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::RunPlaybook {
                        playbook: "../evil.yaml".into(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        let proposal = data
            .proposals
            .iter()
            .find(|item| item.id == proposals[0].id)
            .expect("proposal");
        assert_eq!(proposal.status, ProposalStatus::Invalid);
        assert!(proposal.decided_at.is_some());
        assert!(proposal.decided_by.is_some());
        let error = proposal.error.as_deref().unwrap_or_default();
        assert!(
            error.contains("bundled") || error.contains("path"),
            "{error}"
        );
    }

    #[test]
    fn accepting_propose_resolve_merges_and_unresolve_restores_selection() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Resolve Case");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let domain = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        let host = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Hostname,
                "example.com",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("host");
        app.reload().expect("reload");
        let mut run = sample_run(
            investigation_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Same host?",
        );
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    AgentAction::ProposeResolve {
                        entities: vec!["E-1".into(), "E-2".into()],
                        reason: "same host".into(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept");
        let workspace = app.workspace.as_ref().expect("workspace");
        assert!(workspace.data.is_absorbed(host));
        assert_eq!(workspace.canonical_id(host), domain);
        let view = crate::model::ObjectView::for_entity(
            workspace,
            workspace.entity(domain).expect("canonical"),
        );
        assert!(!view.constituents.is_empty());
        app.view = View::Entities;
        app.entity_id = Some(domain);
        app.dispatch(Action::Unresolve).expect("propose split");
        assert!(matches!(app.overlay, Overlay::Confirm { .. }));
        app.dispatch(Action::Cancel).expect("esc");
        assert_eq!(app.overlay, Overlay::None);
        assert!(
            app.workspace
                .as_ref()
                .expect("workspace")
                .data
                .is_absorbed(host),
            "Esc must not unresolve"
        );
        app.dispatch(Action::Unresolve).expect("propose again");
        app.dispatch(Action::Select).expect("confirm");
        let restored = app.workspace.as_ref().expect("workspace");
        assert!(!restored.data.is_absorbed(host));
        assert_eq!(app.entity_id, Some(domain));
    }

    #[test]
    fn entity_mark_enter_and_esc_restore_do_not_stale_selection() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Marks");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let domain = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        let ip = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.10",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("ip");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::ToggleMergeMark).expect("mark first");
        app.dispatch(Action::Move(1)).expect("down");
        app.dispatch(Action::ToggleMergeMark).expect("mark second");
        assert_eq!(app.merge_marks.len(), 2);
        app.dispatch(Action::ProposeMerge).expect("propose");
        assert_eq!(app.view, View::Review);
        assert_eq!(app.pending_merges.len(), 1);
        assert_eq!(app.merge_marks.len(), 2);
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept merge");
        let workspace = app.workspace.as_ref().expect("workspace");
        assert!(workspace.data.is_absorbed(ip) || workspace.data.is_absorbed(domain));
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.remap_focus_after_resolution();
        app.dispatch(Action::Select).expect("enter object view");
        assert_eq!(app.view, View::Object);
        assert!(app.entity_id.is_some());
        app.dispatch(Action::Back).expect("esc object view");
        assert_eq!(app.view, View::Entities);
        let visible = app.listed_entities();
        assert!(!visible.is_empty());
        assert!(app.selected < visible.len());
        assert!(
            visible
                .iter()
                .any(|entity| Some(entity.id) == app.entity_id)
        );
    }

    #[test]
    fn view_change_clears_marks_and_m_does_not_add_unmarked_highlight() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Mark Leak");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let first = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "alpha.example",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("first");
        let second = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.10",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("second");
        let third = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Email,
                "a@example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("third");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 1;
        app.dispatch(Action::ToggleMergeMark).expect("mark second");
        app.selected = 2;
        app.dispatch(Action::ToggleMergeMark).expect("mark third");
        assert_eq!(app.merge_marks, vec![second, third]);
        app.dispatch(Action::OpenSources).expect("leave entities");
        assert!(
            app.merge_marks.is_empty(),
            "marks must not survive a view change"
        );
        app.dispatch(Action::OpenEntities)
            .expect("back to entities");
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, 0);
        assert!(app.merge_marks.is_empty());
        app.dispatch(Action::ProposeMerge).expect("m");
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.pending_resolve.is_none());
        assert!(app.status.contains("Mark at least two"));
        let listed = app.listed_entities();
        assert_eq!(listed[0].id, first);
    }

    #[test]
    fn open_graph_clears_merge_marks() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Graph Marks");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.10",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("ip");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::ToggleMergeMark).expect("mark");
        assert_eq!(app.merge_marks.len(), 1);
        app.dispatch(Action::OpenGraph).expect("g");
        assert!(app.merge_marks.is_empty());
        assert_eq!(app.view, View::Graph);
    }

    #[test]
    fn merge_canonical_is_highlighted_marked_row_not_leftover_entity_id() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Canonical Highlight");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let first = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "alpha.example",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("first");
        let second = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.11",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("second");
        let leftover = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Email,
                "left@example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("leftover");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::ToggleMergeMark).expect("mark first");
        app.selected = 1;
        app.dispatch(Action::ToggleMergeMark).expect("mark second");
        app.selected = 1;
        app.entity_id = Some(leftover);
        app.dispatch(Action::ProposeMerge).expect("m");
        let pending = app.pending_merges.last().expect("pending");
        assert_eq!(pending.canonical_id, second);
        assert_eq!(pending.entity_ids, vec![first, second]);
        app.pending_merges.clear();
        app.view = View::Entities;
        app.selected = 2;
        app.entity_id = Some(leftover);
        app.dispatch(Action::ProposeMerge)
            .expect("m unmarked highlight");
        let pending = app.pending_merges.last().expect("pending first mark");
        assert_eq!(pending.canonical_id, first);
        assert!(!pending.entity_ids.contains(&leftover));
    }

    #[test]
    fn suggestions_overlay_does_not_auto_merge() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Suggest");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Username,
                "octocat",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("user");
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Url,
                "https://github.com/octocat",
                crate::model::Provenance::External,
                None,
            )
            .expect("url");
        app.reload().expect("reload");
        app.dispatch(Action::OpenSuggestions).expect("suggestions");
        assert_eq!(app.view, View::Review);
        assert!(!app.listed_review_items().is_empty());
        assert!(
            app.workspace
                .as_ref()
                .expect("workspace")
                .data
                .resolutions
                .is_empty()
        );
        app.dispatch(Action::Cancel).expect("esc");
        assert_eq!(app.overlay, Overlay::None);
        assert!(
            app.workspace
                .as_ref()
                .expect("workspace")
                .data
                .resolutions
                .is_empty()
        );
    }

    #[test]
    fn ingest_form_esc_restores_entity_selection() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let mut app = App::new(Config::default(), paths, store);
        create_named_investigation(&mut app, "Ingest Sel");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "alpha.example",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("a");
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.44",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("b");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 1;
        let listed = app.workspace.as_ref().expect("ws").visible_entities();
        let focused = listed.get(1).map(|entity| entity.id);
        app.entity_id = focused;
        app.dispatch(Action::Command("ingest")).expect("i");
        assert!(matches!(
            app.overlay,
            Overlay::Form(ref form) if form.kind == FormKind::Ingest
        ));
        app.dispatch(Action::Cancel).expect("esc");
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, 1);
        assert_eq!(app.entity_id, focused);
        let fixture = directory.path().join("obs.csv");
        std::fs::write(&fixture, "kind,value\ndomain,beta.example\n").expect("csv");
        app.dispatch(Action::Command("ingest")).expect("i again");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("csv");
            form.fields[1].value.set(fixture.display().to_string());
        }
        app.dispatch(Action::Submit).expect("enter");
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, 1);
        let workspace = app.workspace.as_ref().expect("workspace");
        assert!(
            workspace
                .data
                .entities
                .iter()
                .any(|entity| entity.value == "beta.example")
        );
        if let Some(id) = focused {
            assert_eq!(app.entity_id, Some(workspace.canonical_id(id)));
        }
    }

    #[test]
    fn ingest_form_keeps_overlay_on_empty_path_and_invalid_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let mut app = App::new(Config::default(), paths, store);
        create_named_investigation(&mut app, "Ingest Valid");
        app.dispatch(Action::Command("ingest")).expect("i");
        app.dispatch(Action::Submit).expect("empty enter");
        assert!(
            matches!(
                app.overlay,
                Overlay::Form(ref form) if form.kind == FormKind::Ingest
            ),
            "empty path must leave the ingest form open"
        );
        assert!(app.status.to_lowercase().contains("path"));
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[1].value.set("/no/such/ingest-file.json");
        }
        app.dispatch(Action::Submit).expect("missing enter");
        assert!(
            matches!(
                app.overlay,
                Overlay::Form(ref form) if form.kind == FormKind::Ingest
            ),
            "a missing ingest file must leave the form open"
        );
        assert!(!app.status.is_empty());
    }

    #[test]
    fn playbook_form_esc_and_enter_restore_entity_selection() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let mut app = App::new(Config::default(), paths, store);
        create_named_investigation(&mut app, "Playbook Sel");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "alpha.example",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("a");
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Ip,
                "192.0.2.55",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("b");
        app.reload().expect("reload");
        app.view = View::Entities;
        app.pane = Pane::Main;
        app.selected = 1;
        let listed = app.workspace.as_ref().expect("ws").visible_entities();
        let focused = listed.get(1).map(|entity| entity.id);
        app.entity_id = focused;
        app.dispatch(Action::Command("run_playbook")).expect("b");
        assert!(matches!(
            app.overlay,
            Overlay::Form(ref form) if form.kind == FormKind::Playbook
        ));
        app.dispatch(Action::Cancel).expect("esc");
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, 1);
        assert_eq!(app.entity_id, focused);
        app.dispatch(Action::Command("run_playbook"))
            .expect("b again");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("domain-infra");
            form.fields[1].value.set("example.com");
            form.fields[2].value.set("dry-run");
        }
        app.dispatch(Action::Submit).expect("enter");
        assert!(matches!(app.overlay, Overlay::Text { .. }));
        assert_eq!(app.view, View::Entities);
        assert_eq!(app.selected, 1);
        if let Some(id) = focused {
            let workspace = app.workspace.as_ref().expect("workspace");
            assert_eq!(app.entity_id, Some(workspace.canonical_id(id)));
        }
        let workspace = app.workspace.as_ref().expect("workspace");
        assert!(workspace.data.proposals.is_empty());
        assert!(
            workspace
                .data
                .entities
                .iter()
                .all(|entity| { entity.value == "alpha.example" || entity.value == "192.0.2.55" })
        );
        app.dispatch(Action::Cancel).expect("close dry-run");
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn playbook_prompt_seeds_agent_question() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let mut app = App::new(Config::default(), paths, store);
        create_named_investigation(&mut app, "Prompt");
        app.dispatch(Action::Command("playbook_prompt"))
            .expect("cmd");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("domain-infra");
            form.fields[1].value.set("example.com");
        }
        app.dispatch(Action::Submit).expect("enter");
        assert!(matches!(app.overlay, Overlay::AgentPrompt { .. }));
        if let Overlay::AgentPrompt { input } = &app.overlay {
            assert!(input.value.contains("domain-infra"));
            assert!(input.value.contains("RUN_COLLECTOR"));
            assert!(input.value.contains("ASSERT_FINDING_TEMPLATE"));
        }
    }

    #[test]
    fn watch_dry_run_does_not_apply() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let store = Store::open(&paths.database()).expect("store");
        let mut app = App::new(Config::default(), paths, store);
        create_named_investigation(&mut app, "Watch");
        app.dispatch(Action::Command("watch_playbook"))
            .expect("cmd");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("domain-infra");
            form.fields[1].value.set("example.com");
            form.fields[2].value.set("dry-run");
        }
        app.dispatch(Action::Submit).expect("enter");
        assert!(matches!(app.overlay, Overlay::Text { .. }));
        if let Overlay::Text { body, .. } = &app.overlay {
            assert!(body.contains("automation dry-run") || body.contains("Nothing was applied"));
            assert!(
                !body.contains("Observation changed"),
                "watch dry-run must not always list a watch finding"
            );
        }
        let workspace = app.workspace.as_ref().expect("workspace");
        assert!(workspace.data.sources.is_empty());
        assert!(workspace.data.proposals.is_empty());
    }

    fn seed_temporal_geo(app: &mut App) {
        create_named_investigation(app, "Temporal");
        create_page(app, "192.0.2.1", "ip", "192.0.2.1");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page_id = app.workspace.as_ref().expect("workspace").data.pages[0].id;
        let mut batch = crate::model::EvidenceBatch::new();
        let source = crate::model::NewSource::collected(
            crate::model::SourceKind::IpIntel,
            "ip",
            "IP intelligence",
        );
        let key = batch.add_source(source);
        let alpha = batch.entity(crate::model::EntityKind::Ip, "192.0.2.1", Some(key));
        let beta = batch.entity(crate::model::EntityKind::Ip, "192.0.2.8", Some(key));
        let asn = batch.entity(crate::model::EntityKind::Asn, "AS64496", Some(key));
        batch.annotate(&alpha, "country", "US");
        batch.annotate(&alpha, "prefix", "192.0.2.0/24");
        batch.annotate(&alpha, "latitude", "38.0");
        batch.annotate(&alpha, "longitude", "-97.0");
        batch.annotate(&alpha, "geo_source", "collected");
        batch.annotate(&beta, "country", "US");
        batch.annotate(&asn, "prefix", "192.0.2.0/24");
        let started = crate::model::now();
        batch.event(
            started,
            crate::model::TimelineKind::Registration,
            "collected",
            Some(key),
            Some(alpha.clone()),
        );
        batch.case_event(crate::model::NewEvent {
            started_at: started,
            ended_at: Some(started + chrono::Duration::hours(2)),
            kind: crate::model::TimelineKind::Observed,
            title: "imported".to_string(),
            description: String::new(),
            source: Some(key),
            entities: vec![beta.clone()],
            fields: crate::model::Fields::new(),
        });
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: Some(page_id),
                    job_id: None,
                },
                &batch,
            )
            .expect("apply");
        app.reload().expect("reload");
    }

    #[test]
    fn timeline_overlay_filters_and_enter_esc_restore() {
        let (_directory, mut app) = test_app();
        seed_temporal_geo(&mut app);
        app.dispatch(Action::OpenTimeline).expect("timeline");
        app.pane = Pane::Main;
        let rows = app.listed_timeline_rows();
        assert!(rows.iter().any(|row| row.title == "collected"));
        assert!(
            rows.iter()
                .any(|row| row.title == "imported" && row.ended_at.is_some())
        );
        assert!(rows.len() >= 2);
        let collected = rows
            .iter()
            .position(|row| row.title == "collected")
            .expect("collected row");
        app.selected = collected;
        app.sync_timeline_selection();
        app.dispatch(Action::Select).expect("enter");
        assert_eq!(app.view, View::Object);
        app.dispatch(Action::Back).expect("esc");
        assert_eq!(app.view, View::Evidence);
        assert!(app.time_lens);
        assert_eq!(app.selected, collected);
        let candidates = app.timeline_filter_candidates();
        assert!(!candidates.is_empty());
        app.timeline.facet_index = 0;
        app.dispatch(Action::TimelineApplyFilter).expect("filter");
        assert!(app.timeline.filter_entity_id.is_some());
        assert!(!app.listed_timeline_rows().is_empty());
        app.dispatch(Action::Back).expect("clear filter");
        assert!(app.timeline.filter_entity_id.is_none());
        assert_eq!(app.selected, collected);
    }

    #[test]
    fn nearby_and_empty_timeline_do_not_stale_selection() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Empty");
        app.dispatch(Action::OpenTimeline).expect("timeline");
        app.pane = Pane::Main;
        app.dispatch(Action::Select).expect("enter empty");
        assert_eq!(app.view, View::Evidence);
        assert!(app.time_lens);
        app.dispatch(Action::ToggleNearby).expect("nearby empty");
        assert!(!app.timeline.nearby);
        seed_temporal_geo(&mut app);
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        let ip_index = app
            .listed_entities()
            .iter()
            .position(|entity| entity.value == "192.0.2.1")
            .expect("ip");
        app.selected = ip_index;
        app.dispatch(Action::ToggleNearby).expect("nearby");
        assert!(app.timeline.nearby);
        assert!(
            app.listed_nearby()
                .iter()
                .any(|entity| entity.value == "192.0.2.8" || entity.value == "AS64496")
        );
        app.dispatch(Action::Back).expect("esc nearby");
        assert!(!app.timeline.nearby);
        assert_eq!(app.selected, ip_index);
        let collection =
            crate::model::geojson_collection(&app.workspace.as_ref().expect("workspace").data);
        assert!(collection.features.iter().any(|feature| {
            feature
                .geometry
                .as_ref()
                .is_some_and(|geometry| geometry.geometry_type == "Point")
        }));
        assert!(collection.features.iter().any(|feature| {
            feature
                .properties
                .get("prefix")
                .and_then(|value| value.as_str())
                == Some("192.0.2.0/24")
        }));
    }

    #[test]
    fn settings_rejects_unknown_theme() {
        let (_directory, mut app) = test_app();
        app.view = View::Settings;
        app.dispatch(Action::Edit).expect("form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[3].value.set("neon");
        }
        let error = app.dispatch(Action::Submit).expect_err("invalid theme");
        assert!(error.to_string().contains("unknown theme"), "{error}");
        assert_eq!(app.config.ui.theme, "auto");
    }

    #[test]
    fn settings_accepts_known_theme() {
        let (_directory, mut app) = test_app();
        app.view = View::Settings;
        app.dispatch(Action::Edit).expect("form");
        if let Overlay::Form(form) = &mut app.overlay {
            form.fields[3].value.set("phosphor");
        }
        app.dispatch(Action::Submit).expect("save");
        assert_eq!(app.config.ui.theme, "phosphor");
    }

    #[test]
    fn tools_enter_opens_y_confirm_and_does_not_spawn() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Tools Confirm");
        app.dispatch(Action::OpenTools).expect("tools");
        assert_eq!(app.view, View::Tools);
        assert!(!app.tools.tools().is_empty());
        let name = app.tools.tools()[0].definition.name.clone();
        app.pane = Pane::Main;
        app.selected = 0;
        app.dispatch(Action::Select).expect("enter");
        assert!(
            matches!(
                &app.overlay,
                Overlay::Confirm { command, .. } if command == &format!("run_tool:{name}")
            ),
            "{:?}",
            app.overlay
        );
        assert!(app.job_records.is_empty());
        app.dispatch(Action::Cancel).expect("esc");
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.job_records.is_empty());
    }

    #[test]
    fn tools_enter_on_an_active_tool_sets_error_and_does_not_enable() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Tools Refused");
        app.config
            .tools
            .custom
            .push(crate::config::ToolDefinitionConfig {
                name: "nmap".into(),
                program: "nmap".into(),
                args: vec!["{target}".into()],
                accepts: vec!["ip".into()],
                description: "active probe".into(),
                activity: "active".into(),
                ..Default::default()
            });
        app.tools = Arc::new(crate::tools::ToolRegistry::detect(&app.config.tools));
        app.dispatch(Action::OpenTools).expect("tools");
        let index = app
            .tools
            .tools()
            .iter()
            .position(|entry| entry.definition.name == "nmap")
            .expect("nmap listed");
        app.pane = Pane::Main;
        app.selected = index;
        app.dispatch(Action::Select).expect("enter");
        assert_eq!(app.overlay, Overlay::None, "no enable or confirm path");
        let error = app.last_error.as_deref().unwrap_or("");
        assert!(
            error.contains("refused") || error.contains("nmap"),
            "{error}"
        );
        assert!(app.job_records.is_empty());
    }

    #[test]
    fn board_jk_syncs_inspector_entity_and_page() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Inspector Follow");
        create_page(&mut app, "alpha.example", "domain", "alpha.example");
        create_page(&mut app, "beta.example", "domain", "beta.example");
        app.reload().expect("reload");
        app.dispatch(Action::OpenOverview).expect("case");
        app.pane = Pane::Main;
        app.selected = 0;
        app.sync_board_selection();
        let first_page = app.page_id;
        app.dispatch(Action::Move(1)).expect("j");
        assert_eq!(app.selected, 1);
        assert_ne!(app.page_id, first_page);
        assert_eq!(
            app.page_id,
            app.workspace.as_ref().and_then(|workspace| workspace
                .data
                .pages
                .get(1)
                .map(|page| page.id))
        );

        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Main;
        app.selected = 0;
        app.sync_board_selection();
        let first_entity = app.entity_id;
        app.dispatch(Action::Move(1)).expect("j");
        assert_eq!(app.selected, 1);
        assert_ne!(app.entity_id, first_entity);
        assert_eq!(
            app.entity_id,
            app.listed_entities().get(1).map(|entity| entity.id)
        );

        app.dispatch(Action::OpenEvidence).expect("evidence");
        app.pane = Pane::Main;
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page_a = app.workspace.as_ref().expect("workspace").data.pages[0].id;
        let page_b = app.workspace.as_ref().expect("workspace").data.pages[1].id;
        app.store
            .create_finding(crate::storage::NewFinding {
                investigation_id,
                page_id: Some(page_a),
                title: "first finding".into(),
                details: "a".into(),
                confidence: crate::model::Confidence::Low,
                provenance: crate::model::Provenance::Analyst,
                source_ids: Vec::new(),
            })
            .expect("finding a");
        app.store
            .create_finding(crate::storage::NewFinding {
                investigation_id,
                page_id: Some(page_b),
                title: "second finding".into(),
                details: "b".into(),
                confidence: crate::model::Confidence::High,
                provenance: crate::model::Provenance::Analyst,
                source_ids: Vec::new(),
            })
            .expect("finding b");
        app.reload().expect("reload");
        app.dispatch(Action::OpenEvidence).expect("evidence again");
        app.pane = Pane::Main;
        app.selected = 0;
        app.sync_board_selection();
        let first_finding = app.finding_id;
        app.dispatch(Action::Move(1)).expect("j on evidence");
        assert_eq!(app.selected, 1);
        assert_ne!(app.finding_id, first_finding);
        assert_eq!(app.page_id, Some(page_b));
    }

    #[test]
    fn switching_views_resets_board_status_and_one_rail_row() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Rail Current");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = Pane::Nav;
        app.status = "stale".into();
        app.dispatch(Action::OpenEvidence).expect("evidence");
        assert_eq!(app.view, View::Evidence);
        assert_eq!(app.pane, Pane::Main);
        assert_ne!(app.status, "stale");
        assert!(app.status.contains("Evidence"));
        assert!(app.rail_view_is_current(View::Evidence));
        assert!(!app.rail_view_is_current(View::Entities));
        assert!(!app.rail_view_is_current(View::Overview));
        assert_eq!(
            app.nav_items()
                .iter()
                .filter(|(_, view)| app.rail_view_is_current(*view))
                .count(),
            1
        );
        app.pane = Pane::Detail;
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        assert_eq!(app.pane, Pane::Main);
        assert!(app.rail_view_is_current(View::Graph));
        assert!(!app.rail_view_is_current(View::Evidence));
    }

    #[test]
    fn graph_t_toggles_time_lens_on_the_same_view() {
        let (_directory, mut app) = test_app();
        seed_temporal_geo(&mut app);
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        assert!(!app.is_time_lens());
        app.dispatch(Action::ToggleTimeLens).expect("t");
        assert_eq!(app.view, View::Graph);
        assert!(app.time_lens);
        assert!(app.is_time_lens());
        app.dispatch(Action::ToggleTimeLens).expect("t off");
        assert_eq!(app.view, View::Graph);
        assert!(!app.is_time_lens());
    }

    #[test]
    fn agent_history_and_search_render_the_run_in_review() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Agent History");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let thread = Uuid::new_v4();
        let mut run = sample_run(investigation_id, Uuid::new_v4(), thread, "Who owns this?");
        run.response = "The registrar is Example".into();
        run.status = JobStatus::Succeeded;
        app.store.insert_agent_run(&mut run).expect("run");
        app.reload().expect("reload");
        app.agent_run_id = Some(run.id);
        app.dispatch(Action::OpenAgent).expect("history");
        assert_eq!(app.view, View::Review);
        assert_eq!(app.agent_run_id, Some(run.id));
        assert_eq!(app.agent_thread, Some(thread));
        let item = app.selected_review_item().expect("history row");
        assert_eq!(item.kind, crate::app::review::ReviewKind::History);
        assert_eq!(item.run_id, Some(run.id));
        assert!(item.written_as.is_none());
        let hits = app.search_hits("Who owns");
        assert!(
            hits.iter()
                .any(|hit| hit.id == run.id && hit.view == View::Review)
        );
    }

    #[test]
    fn review_preview_matches_write_and_shares_citation_resolve() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Preview Match");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut batch = crate::model::EvidenceBatch::new();
        let mut source = crate::model::NewSource::collected(
            crate::model::SourceKind::Dns,
            "dns",
            "DNS for preview",
        );
        source.content = "A 203.0.113.10".into();
        batch.add_source(source);
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: None,
                    job_id: None,
                },
                &batch,
            )
            .expect("source");
        app.reload().expect("source reload");
        let stored_id = app.workspace.as_ref().expect("workspace").data.sources[0].id;
        let mut playbook_run = sample_run(
            investigation_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "playbook template",
        );
        playbook_run.agent_name = crate::storage::PLAYBOOK_AGENT_NAME.into();
        playbook_run.status = JobStatus::Succeeded;
        app.store.insert_agent_run(&mut playbook_run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                playbook_run.id,
                vec![(
                    AgentAction::AssertFindingTemplate {
                        title: "templated finding".into(),
                        details: "from playbook".into(),
                    },
                    ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.reload().expect("reload");
        app.open_review_lane();
        let item = app.selected_review_item().expect("item");
        assert_eq!(item.written_as, Some(Provenance::Analyst));
        assert!(!item.is_ai());
        assert!(
            item.preview.iter().any(|line| line.contains("analyst")),
            "{:?}",
            item.preview
        );
        assert!(app.confirm_accept_text().contains("analyst"));
        assert!(app.confirm_bar_text().contains("analyst"));
        let workspace = app.workspace.as_ref().expect("workspace");
        assert_eq!(
            crate::app::review::resolve_source_citation(workspace, "S-1"),
            Some(stored_id)
        );
        assert_eq!(
            crate::app::review::resolve_source_citation(workspace, "DNS for preview"),
            Some(stored_id)
        );
        app.dispatch(Action::Arm).expect("arm");
        app.dispatch(Action::Accept).expect("accept");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("loads");
        assert_eq!(data.findings[0].title, "templated finding");
        assert_eq!(data.findings[0].provenance, Provenance::Analyst);
    }

    #[test]
    fn reject_dismisses_scored_merge_and_keeps_selection_valid() {
        let (directory, mut app) = test_app();
        create_named_investigation(&mut app, "Reject Persist");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Username,
                "octocat",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("user");
        app.store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Url,
                "https://github.com/octocat",
                crate::model::Provenance::External,
                None,
            )
            .expect("url");
        app.reload().expect("reload");
        app.open_review_lane();
        assert!(!app.listed_suggestions().is_empty());
        let before = app.listed_review_items().len();
        let merge_index = app
            .listed_review_items()
            .iter()
            .position(|item| item.merge.is_some())
            .expect("merge row");
        app.selected = merge_index;
        app.dispatch(Action::Reject).expect("reject");
        assert!(app.listed_suggestions().is_empty());
        assert!(
            app.listed_review_items()
                .iter()
                .all(|item| item.merge.is_none())
        );
        assert!(app.listed_review_items().len() < before || before == 0);
        let remaining = app.listed_review_items().len();
        if remaining == 0 {
            assert_eq!(app.selected, 0);
        } else {
            assert!(app.selected < remaining);
        }
        app.dispatch(Action::Quit).expect("save session");
        let paths = AppPaths::new(
            directory.path().join("config.toml"),
            directory.path().join("data"),
        );
        let store = Store::open(&paths.database()).expect("reopen");
        let mut restarted = App::new(Config::default(), paths, store);
        restarted.open_last_or_welcome().expect("restore");
        restarted.open_review_lane();
        assert!(
            restarted.listed_suggestions().is_empty(),
            "rejected suggestions must stay dismissed after restart"
        );
    }

    #[test]
    fn switching_cases_with_identical_counts_shows_the_new_case_rows() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Case Alpha");
        create_page(&mut app, "alpha.example", "domain", "alpha.example");
        let alpha_id = app.workspace.as_ref().expect("workspace").id();
        app.dispatch(Action::OpenEntities).expect("entities");
        let alpha = app.listed_entities()[0].value.clone();

        create_named_investigation(&mut app, "Case Beta");
        create_page(&mut app, "beta.example", "domain", "beta.example");
        app.dispatch(Action::OpenEntities).expect("entities");
        let beta = app.listed_entities()[0].value.clone();
        assert_ne!(alpha, beta);
        assert!(beta.contains("beta"), "{beta}");

        app.open_investigation(alpha_id).expect("switch");
        app.dispatch(Action::OpenEntities).expect("entities");
        let restored = app.listed_entities()[0].value.clone();
        assert_eq!(restored, alpha);
        assert!(
            !app.listed_entities()
                .iter()
                .any(|entity| entity.value == beta)
        );
    }

    #[test]
    fn accepting_a_merge_does_not_duplicate_board_rows() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Merge Cache");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let domain = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Domain,
                "example.com",
                Provenance::Analyst,
                None,
            )
            .expect("domain");
        let host = app
            .store
            .add_entity(
                investigation_id,
                crate::model::EntityKind::Hostname,
                "example.com",
                Provenance::Collected,
                None,
            )
            .expect("host");
        app.reload().expect("reload");
        app.dispatch(Action::OpenEntities).expect("entities");
        assert_eq!(app.listed_entities().len(), 2);

        let actor = app.store.identity().user_id.clone();
        app.store
            .resolve_entities(
                investigation_id,
                &[domain, host],
                domain,
                "same host",
                &actor,
            )
            .expect("resolve");
        let data = app
            .store
            .load_investigation(investigation_id)
            .expect("load");
        app.workspace = Some(Workspace::new(data));
        let listed = app.listed_entities();
        let unique: HashSet<_> = listed.iter().map(|entity| entity.id).collect();
        assert_eq!(listed.len(), unique.len(), "canonical rows must be unique");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, domain);
    }

    #[test]
    fn job_status_change_is_reflected_under_a_text_filter() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Job Filter");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut job = sample_job("DNS", JobStatus::Queued, None);
        job.investigation_id = Some(investigation_id);
        app.store.insert_job(&mut job).expect("store job");
        app.job_records.insert(job.id, job.clone());
        app.view = View::Jobs;
        app.board_filter = "running".into();
        app.handle_job(JobEvent::Started { job_id: job.id })
            .expect("started");
        assert_eq!(app.listed_job_ids(), vec![job.id]);
        app.handle_job(JobEvent::Finished {
            job_id: job.id,
            outcome: JobOutcome::Failed("nope".into()),
        })
        .expect("failed");
        assert!(
            app.listed_job_ids().is_empty(),
            "failed jobs must drop out of a running filter"
        );
    }

    #[test]
    fn job_log_ticks_do_not_rebuild_board_lists() {
        let (_directory, mut app) = test_app();
        create_named_investigation(&mut app, "Log Ticks");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut job = sample_job("DNS", JobStatus::Running, None);
        job.investigation_id = Some(investigation_id);
        app.store.insert_job(&mut job).expect("store job");
        app.job_records.insert(job.id, job.clone());
        app.view = View::Entities;
        let _ = app.listed_entities();
        let rev = app.workspace_rev;
        app.handle_job(JobEvent::Log {
            job_id: job.id,
            line: "looking up".into(),
            is_error: false,
        })
        .expect("log");
        assert_eq!(
            app.workspace_rev, rev,
            "log ticks must not bump workspace_rev"
        );
        app.handle_job(JobEvent::Progress {
            job_id: job.id,
            message: "page 1/8".into(),
        })
        .expect("progress");
        assert_eq!(app.workspace_rev, rev);
    }
}
