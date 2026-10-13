//! User and system actions. The TUI maps keys to these; `App::dispatch` applies them.

use uuid::Uuid;

use crate::jobs::JobEvent;

#[derive(Debug)]
pub enum Action {
    Quit,
    Cancel,
    Tick,
    NextPane,
    PrevPane,
    Move(i32),
    Select,
    Back,
    Help,
    Palette,
    Search,
    AskAgent,
    NewItem,
    Edit,
    Refresh,
    OpenJobs,
    OpenTools,
    OpenSettings,
    OpenPages,
    OpenSources,
    OpenFindings,
    OpenEntities,
    OpenTimeline,
    OpenAgent,
    OpenOverview,
    Command(&'static str),
    Submit,
    Cycle(i32),
    Accept,
    Reject,
    Job(JobEvent),
    InsertChar(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Scroll(i32),
    // Internal: loaded investigation id to open after create.
    OpenInvestigation(Uuid),
}
