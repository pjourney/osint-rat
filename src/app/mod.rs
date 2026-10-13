//! Application state and action handling. Testable without a terminal.

pub mod action;
pub mod commands;
pub mod input;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, WrapErr, eyre};
use uuid::Uuid;

pub use action::Action;
pub use input::Input;

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
    Page, PageKind, Proposal, ProposalStatus, Provenance, Target, Workspace, now,
};
use crate::paths::AppPaths;
use crate::report::write_report;
use crate::storage::{ApplyContext, GLOBAL_SCOPE, NewFinding, NewPage, Store};
use crate::tools::{Activity, ToolEntry, ToolRegistry};

use self::commands::filtered;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Welcome,
    Overview,
    Pages,
    Sources,
    Findings,
    Entities,
    Timeline,
    Notes,
    Agent,
    Jobs,
    Tools,
    Settings,
    Artifacts,
}

impl View {
    pub fn as_str(self) -> &'static str {
        match self {
            View::Welcome => "welcome",
            View::Overview => "overview",
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
        }
    }

    pub fn parse(value: &str) -> Option<View> {
        Some(match value {
            "welcome" => View::Welcome,
            "overview" => View::Overview,
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
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Nav,
    Main,
    Detail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Palette {
        query: Input,
        selected: usize,
    },
    Search {
        query: Input,
        selected: usize,
    },
    AgentPrompt {
        input: Input,
    },
    Form(FormState),
    Confirm {
        title: String,
        message: String,
        command: String,
    },
    Text {
        title: String,
        body: String,
        scroll: u16,
    },
    Doctor,
    Proposals {
        run_id: Uuid,
        selected: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    NewInvestigation,
    OpenInvestigation,
    NewPage,
    NewFinding,
    NewNote,
    SettingsAgent,
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
    pub status: String,
    pub dirty: bool,
    pub should_quit: bool,
    pub agent: AgentStatus,
    pub streaming: HashMap<Uuid, String>,
    pub last_error: Option<String>,
    pub pending_editor: Option<EditorSession>,
}

/// External `$EDITOR` session for long-form page notes.
#[derive(Clone, Debug)]
pub struct EditorSession {
    pub path: PathBuf,
    pub page_id: Uuid,
}

impl App {
    pub fn new(config: Config, paths: AppPaths, store: Store) -> Self {
        let tools = Arc::new(ToolRegistry::detect(&config.tools));
        let agent = AgentStatus::resolve(&config, &tools);
        let net = NetContext::new(&config.network).map(Arc::new);
        let max_jobs = config.collectors.max_concurrent_jobs.max(1);
        let app = Self {
            config,
            paths,
            store,
            tools,
            collectors: CollectorRegistry::builtin(),
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
            status: "Press [?] for help, [p] for commands.".into(),
            dirty: true,
            should_quit: false,
            agent,
            streaming: HashMap::new(),
            last_error: None,
            pending_editor: None,
        };
        let _ = app.store.mark_interrupted_jobs();
        app
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn investigations(&self) -> Vec<crate::model::InvestigationSummary> {
        self.store.list_investigations().unwrap_or_default()
    }

    pub fn nav_items(&self) -> Vec<(&'static str, View)> {
        vec![
            ("Overview", View::Overview),
            ("Pages", View::Pages),
            ("Sources", View::Sources),
            ("Findings", View::Findings),
            ("Entities", View::Entities),
            ("Timeline", View::Timeline),
            ("Notes", View::Notes),
            ("Artifacts", View::Artifacts),
            ("Agent", View::Agent),
            ("Jobs", View::Jobs),
            ("Tools", View::Tools),
            ("Settings", View::Settings),
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
        self.view = View::Overview;
        self.page_id = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.data.pages.first().map(|page| page.id));
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
        if let Some(view) = self.store.get_state(&scope, "view")?
            && let Some(view) = View::parse(&view)
            && view != View::Welcome
        {
            self.view = view;
            if let Some(index) = self
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
        if let Some(thread) = self.agent_thread {
            self.store
                .set_state(&scope, "agent_thread", &thread.to_string())?;
        }
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
        self.mark_dirty();
        Ok(())
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
            Action::Tick => self.mark_dirty(),
            Action::NextPane => self.cycle_pane(1),
            Action::PrevPane => self.cycle_pane(-1),
            Action::Move(delta) => self.move_selection(delta),
            Action::Select => self.select()?,
            Action::Back => self.cancel(),
            Action::Help => {
                self.scroll = 0;
                self.overlay = Overlay::Help;
            }
            Action::Palette => {
                self.overlay = Overlay::Palette {
                    query: Input::default(),
                    selected: 0,
                }
            }
            Action::Search => {
                self.overlay = Overlay::Search {
                    query: Input::default(),
                    selected: 0,
                }
            }
            Action::AskAgent => self.open_agent_prompt(),
            Action::NewItem => self.open_new()?,
            Action::Edit => self.open_edit()?,
            Action::Refresh => self.run_default_collectors()?,
            Action::OpenJobs => self.switch_view(View::Jobs),
            Action::OpenTools => self.switch_view(View::Tools),
            Action::OpenSettings => self.switch_view(View::Settings),
            Action::OpenPages => self.switch_view(View::Pages),
            Action::OpenSources => self.switch_view(View::Sources),
            Action::OpenFindings => self.switch_view(View::Findings),
            Action::OpenEntities => self.switch_view(View::Entities),
            Action::OpenTimeline => self.switch_view(View::Timeline),
            Action::OpenAgent => self.switch_view(View::Agent),
            Action::OpenOverview => self.switch_view(View::Overview),
            Action::Command(id) => self.run_command(id)?,
            Action::Submit => self.submit()?,
            Action::Cycle(delta) => self.cycle_form(delta),
            Action::Accept => self.decide_proposal(ProposalStatus::Accepted)?,
            Action::Reject => self.decide_proposal(ProposalStatus::Rejected)?,
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
                if self.view != View::Welcome && self.workspace.is_none() {
                    self.view = View::Welcome;
                }
            }
            Overlay::Form(_)
            | Overlay::Palette { .. }
            | Overlay::Search { .. }
            | Overlay::AgentPrompt { .. }
            | Overlay::Confirm { .. }
            | Overlay::Help
            | Overlay::Text { .. }
            | Overlay::Doctor
            | Overlay::Proposals { .. } => {
                self.overlay = Overlay::None;
            }
        }
    }

    fn cycle_pane(&mut self, delta: i32) {
        self.pane = match (self.pane, delta.signum()) {
            (Pane::Nav, 1) | (Pane::Detail, -1) => Pane::Main,
            (Pane::Main, 1) => Pane::Detail,
            (Pane::Main, -1) => Pane::Nav,
            (Pane::Nav, -1) => Pane::Detail,
            (Pane::Detail, 1) => Pane::Nav,
            _ => self.pane,
        };
    }

    fn switch_view(&mut self, view: View) {
        if self.workspace.is_none() && !matches!(view, View::Welcome | View::Settings | View::Tools)
        {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        self.view = view;
        self.selected = 0;
        self.scroll = 0;
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == view)
        {
            self.nav_index = index;
        }
        self.overlay = Overlay::None;
        let _ = self.save_session();
    }

    fn move_selection(&mut self, delta: i32) {
        let search_count = if let Overlay::Search { query, .. } = &self.overlay {
            Some(self.search_hits(&query.value).len().max(1))
        } else {
            None
        };
        let proposal_count = if let Overlay::Proposals { run_id, .. } = &self.overlay {
            Some(self.pending_proposals(*run_id).len().max(1))
        } else {
            None
        };
        match &mut self.overlay {
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
            Overlay::Proposals { selected, .. } => {
                let count = proposal_count.unwrap_or(1);
                *selected = (*selected as i32 + delta).rem_euclid(count as i32) as usize;
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
            | Overlay::AgentPrompt { .. }
            | Overlay::Confirm { .. }
            | Overlay::Doctor => {
                if self.pane == Pane::Nav {
                    let count = self.nav_items().len().max(1);
                    self.nav_index =
                        (self.nav_index as i32 + delta).rem_euclid(count as i32) as usize;
                } else {
                    self.selected = (self.selected as i32 + delta).max(0) as usize;
                }
            }
        }
    }

    fn select(&mut self) -> Result<()> {
        match &self.overlay {
            Overlay::Palette { query, selected } => {
                let items = filtered(&query.value);
                if let Some(command) = items.get(*selected) {
                    let id = command.id;
                    self.overlay = Overlay::None;
                    self.run_command(id)?;
                }
            }
            Overlay::Search { query, selected } => {
                let hits = self.search_hits(&query.value);
                if let Some(hit) = hits.get(*selected).cloned() {
                    self.overlay = Overlay::None;
                    self.open_hit(&hit);
                }
            }
            Overlay::None if self.view == View::Welcome => match self.selected {
                0 => self.open_new()?,
                1 => self.open_open_form(),
                2 => self.overlay = Overlay::Doctor,
                _ => self.overlay = Overlay::Help,
            },
            Overlay::None if self.pane == Pane::Nav => {
                let view = self.nav_items()[self.nav_index].1;
                self.switch_view(view);
            }
            Overlay::None => self.select_main_item(),
            Overlay::Form(_) => self.submit()?,
            Overlay::Confirm { command, .. } => {
                let command = command.clone();
                self.overlay = Overlay::None;
                self.run_command(&command)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn select_main_item(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        match self.view {
            View::Pages => {
                if let Some(page) = workspace.data.pages.get(self.selected) {
                    self.page_id = Some(page.id);
                    self.view = View::Overview;
                    let _ = self.save_session();
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
                if let Some(entity) = workspace.data.entities.get(self.selected) {
                    self.entity_id = Some(entity.id);
                }
            }
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
            View::Jobs => {
                if let Some(job) = workspace.data.jobs.get(self.selected) {
                    let body = format!(
                        "Program: {}\nArguments: {}\nStatus: {}\n\n{}\n\n{}",
                        job.program.as_deref().unwrap_or("-"),
                        job.args.join(" "),
                        job.status,
                        job.log,
                        job.stderr
                    );
                    self.overlay = Overlay::Text {
                        title: format!("{} {}", job.short_ref(), job.description),
                        body,
                        scroll: 0,
                    };
                }
            }
            View::Tools => {
                if let Some(entry) = self.tools.tools().get(self.selected) {
                    let name = entry.definition.name.clone();
                    let _ = self.spawn_tool(&name);
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

    fn open_hit(&mut self, hit: &SearchHit) {
        self.switch_view(hit.view);
        match hit.view {
            View::Pages | View::Overview => self.page_id = Some(hit.id),
            View::Sources => self.source_id = Some(hit.id),
            View::Findings => self.finding_id = Some(hit.id),
            View::Entities => self.entity_id = Some(hit.id),
            View::Agent => {
                self.agent_run_id = Some(hit.id);
                if let Some(thread_id) = self
                    .workspace
                    .as_ref()
                    .and_then(|workspace| workspace.agent_run(hit.id))
                    .map(|run| run.thread_id)
                {
                    self.agent_thread = Some(thread_id);
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
            View::Findings => self.overlay = Overlay::Form(finding_form()),
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
                        "default / high-contrast / monochrome",
                    ),
                ],
            );
            self.overlay = Overlay::Form(form);
            return Ok(());
        }
        if matches!(self.view, View::Notes | View::Overview)
            && let Some(page) = self.current_page().cloned()
        {
            return self.open_external_editor(&page);
        }
        if let Some(page) = self.current_page() {
            self.overlay = Overlay::Form(FormState::new(
                FormKind::NewNote,
                vec![field("Notes", &page.notes, "Markdown")],
            ));
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
            page_id: page.id,
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
        self.store.update_page_notes(session.page_id, &notes)?;
        self.reload()?;
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
                self.reload()?;
                self.view = View::Overview;
                self.status = format!("Created {} — press r to run collectors", page.title);
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
                self.store.create_finding(NewFinding {
                    investigation_id: workspace.id(),
                    page_id: self.page_id,
                    title: values[0].clone(),
                    details: values.get(1).cloned().unwrap_or_default(),
                    confidence,
                    provenance: Provenance::Analyst,
                    source_ids,
                })?;
                self.reload()?;
                self.status = "Finding added.".into();
            }
            FormKind::NewNote => {
                if self.view == View::Overview || self.current_page().is_some() {
                    if let Some(page_id) = self.page_id {
                        self.store
                            .update_page_notes(page_id, values.first().unwrap_or(&String::new()))?;
                    }
                } else if let Some(workspace) = &self.workspace {
                    self.store.create_note(
                        workspace.id(),
                        self.page_id,
                        values.first().unwrap_or(&String::new()),
                        Provenance::Analyst,
                    )?;
                }
                self.reload()?;
            }
            FormKind::SettingsAgent => {
                self.config.agent.active = values[0].clone();
                self.config.agent.program =
                    values.get(1).filter(|value| !value.is_empty()).cloned();
                if let Ok(timeout) = values.get(2).map(String::as_str).unwrap_or("").parse() {
                    self.config.agent.timeout_seconds = timeout;
                }
                if let Some(theme) = values.get(3).filter(|value| !value.is_empty()) {
                    self.config.ui.theme = theme.clone();
                }
                self.config.save(&self.paths.config_file)?;
                self.agent = AgentStatus::resolve(&self.config, &self.tools);
                self.status = format!("Agent set to {}", self.agent.profile.name);
            }
        }
        Ok(())
    }

    fn run_command(&mut self, id: &str) -> Result<()> {
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
            "ask_agent" => self.open_agent_prompt(),
            "generate_report" => self.generate_report("markdown")?,
            "add_finding" => self.overlay = Overlay::Form(finding_form()),
            "add_note" => self.overlay = Overlay::Form(note_form()),
            "open_entities" => self.switch_view(View::Entities),
            "open_sources" => self.switch_view(View::Sources),
            "open_findings" => self.switch_view(View::Findings),
            "open_timeline" => self.switch_view(View::Timeline),
            "open_jobs" => self.switch_view(View::Jobs),
            "open_tools" => self.switch_view(View::Tools),
            "open_agent" => self.switch_view(View::Agent),
            "open_settings" => self.switch_view(View::Settings),
            "open_artifacts" => self.switch_view(View::Artifacts),
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
            && form.kind == FormKind::NewPage
            && form.index == 1
        {
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
            Overlay::AgentPrompt { input } => input.insert(ch),
            Overlay::Form(form) => {
                if let Some(field) = form.current_mut() {
                    field.value.insert(ch);
                }
            }
            _ => {}
        }
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
            Overlay::Palette { query, .. } | Overlay::Search { query, .. } => op(query),
            Overlay::AgentPrompt { input } => op(input),
            Overlay::Form(form) => {
                if let Some(field) = form.current_mut() {
                    op(&mut field.value);
                }
            }
            _ => {}
        }
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
        self.job_records.insert(job_id, record);
        let net_error = self.net.as_ref().err().cloned();
        if collector.requires().is_empty() && net_error.is_some() && !matches!(id, "file") {
            return Err(eyre!(net_error.unwrap_or_default()));
        }
        let tools = Arc::clone(&self.tools);
        let files = self.files().ok_or_else(|| eyre!("no investigation"))?;
        let cache_dir = self.paths.cache_dir();
        let settings = CollectorSettings {
            fetch_robots: self.config.collectors.fetch_robots,
            check_www: self.config.collectors.check_www,
            repository_history_depth: self.config.collectors.repository_history_depth,
            max_file_import_bytes: self.config.storage.max_file_import_bytes,
        };
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
        self.job_records.insert(job_id, record);
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
        self.view = View::Agent;
        self.status = format!("Asking {}…", self.agent.profile.name);
        let _ = run_id;
        Ok(())
    }

    fn focus_ref(&self) -> Option<String> {
        let workspace = self.workspace.as_ref()?;
        let id = match self.view {
            View::Entities => self.entity_id,
            View::Sources => self.source_id,
            View::Findings => self.finding_id,
            _ => None,
        }?;
        workspace.ref_of(id).map(|reference| reference.to_string())
    }

    fn focus_label(&self) -> Option<String> {
        if self.view != View::Entities {
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
                    job.progress = Some(message);
                    let _ = self.store.update_job(job);
                }
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
        self.mark_dirty();
        Ok(())
    }

    fn finish_job(&mut self, job_id: Uuid, outcome: JobOutcome) -> Result<()> {
        self.jobs.finished(job_id);
        let mut result_text = None;
        let mut error_text = None;
        let mut status = JobStatus::Succeeded;
        match outcome {
            JobOutcome::Succeeded(output) => match *output {
                JobOutput::Collection(collected) => {
                    self.apply_collection(job_id, collected)?;
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
        if let Some(error) = error_text {
            self.status = error;
        }
        self.reload()?;
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
        let summary = self.store.apply_batch(
            ApplyContext {
                investigation_id: workspace.id(),
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
            self.overlay = Overlay::Proposals {
                run_id,
                selected: 0,
            };
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

    fn proposals_for(&self, run_id: Uuid) -> Vec<Proposal> {
        self.workspace
            .as_ref()
            .map(|workspace| {
                workspace
                    .proposals_for_run(run_id)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn pending_proposals(&self, run_id: Uuid) -> Vec<Proposal> {
        self.proposals_for(run_id)
            .into_iter()
            .filter(|proposal| proposal.status == ProposalStatus::Pending)
            .collect()
    }

    fn decide_proposal(&mut self, status: ProposalStatus) -> Result<()> {
        let Overlay::Proposals { run_id, selected } = self.overlay else {
            return Ok(());
        };
        let Some(proposal) = self.pending_proposals(run_id).get(selected).cloned() else {
            return Ok(());
        };
        if proposal.status != ProposalStatus::Pending {
            return Ok(());
        }
        if status == ProposalStatus::Accepted
            && let Err(error) = self.apply_proposal(&proposal.action)
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
        let pending_len = self.pending_proposals(run_id).len();
        if pending_len == 0 {
            self.overlay = Overlay::None;
        } else if let Overlay::Proposals { selected, .. } = &mut self.overlay
            && *selected >= pending_len
        {
            *selected = pending_len - 1;
        }
        Ok(())
    }

    fn apply_proposal(&mut self, action: &AgentAction) -> Result<()> {
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
                    .filter_map(|reference| {
                        reference
                            .parse()
                            .ok()
                            .and_then(|parsed| workspace.resolve_ref(parsed))
                    })
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
                    .filter_map(|reference| {
                        reference
                            .parse()
                            .ok()
                            .and_then(|parsed| workspace.resolve_ref(parsed))
                    })
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
        }
        Ok(())
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
                View::Pages,
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
                View::Findings,
            );
        }
        for source in &workspace.data.sources {
            push(
                &mut hits,
                &format!("{} {}", source.title, source.content),
                format!("{} {}", source.short_ref(), source.title),
                "source",
                source.id,
                View::Sources,
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
                View::Agent,
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

    /// Jobs in the order the Jobs pane lists them: persisted rows, then live jobs not reloaded yet.
    pub fn listed_job_ids(&self) -> Vec<Uuid> {
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
        ids
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
            bail_status(&mut self.status, &format!("unknown tool {name}"));
            return Ok(());
        };
        if !entry.is_available() {
            bail_status(
                &mut self.status,
                &format!(
                    "{} is not available: {}",
                    entry.definition.name,
                    entry.problem.as_deref().unwrap_or("not found on PATH")
                ),
            );
            return Ok(());
        }
        if entry.definition.activity == Activity::Active {
            bail_status(
                &mut self.status,
                &format!(
                    "{} is an active probe. os-int only runs passive observation and diagnostic tools.",
                    entry.definition.name
                ),
            );
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
        self.job_records.insert(job_id, record);
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
        app.overlay = Overlay::Proposals {
            run_id: run.id,
            selected: 0,
        };
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
            view: View::Agent,
        });
        assert_eq!(app.agent_run_id, Some(older.id));
        assert_eq!(app.agent_thread, Some(older_thread));
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
}
