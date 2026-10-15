//! Motion engine: one 60 ms tick that exists only while something is animating.
//!
//! Widgets read [`Animator::frame`] and stay pure functions of `(state, frame)`.
//! Snapshot tests freeze any tick with [`Animator::freeze_at`].
//! Ticks never write case data: the Review confirm bar has no timer path to accept.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Config;

use super::glyphs::Glyphs;
use super::theme::{ColorDepth, Role, TerminalEnv, Theme};

/// Duration of one motion tick. All catalogue durations are whole ticks of this length.
pub const TICK_MS: u64 = 60;
/// Tick interval used by the event loop while an animation is active.
pub const TICK: Duration = Duration::from_millis(TICK_MS);
/// More than this many ticks behind degrades Full → Reduced (slow SSH / backed-up tty).
pub const LAG_TICKS_BEFORE_REDUCED: u32 = 3;
/// At most this many toasts are drawn. Older ones collapse to `+N`.
pub const TOAST_STACK: usize = 3;
/// Toast column width on the Board.
pub const TOAST_COLS: u16 = 34;

/// `ui.motion` after environment overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionMode {
    Full,
    Reduced,
    Off,
}

impl MotionMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "full" | "on" | "true" => Some(Self::Full),
            "reduced" | "reduce" | "less" => Some(Self::Reduced),
            "off" | "none" | "false" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reduced => "reduced",
            Self::Off => "off",
        }
    }

    pub fn allows_timer(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn allows_travel(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// One easing curve per job. Progress and spinners stay linear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    EaseOutCubic,
    EaseInQuad,
    Linear,
    Step,
}

impl Easing {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Self::EaseInQuad => t * t,
            Self::Linear => t,
            Self::Step => t,
        }
    }
}

/// Catalogue kinds. Adding one requires a row in [`CATALOGUE`] and in §8.2 of the UX plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimKind {
    Splash,
    JobSpinner,
    AgentPulse,
    Progress,
    Focus,
    Drawer,
    GraphExpand,
    Toast,
    Confirm,
    Radar,
}

impl AnimKind {
    pub const ALL: [AnimKind; 10] = [
        Self::Splash,
        Self::JobSpinner,
        Self::AgentPulse,
        Self::Progress,
        Self::Focus,
        Self::Drawer,
        Self::GraphExpand,
        Self::Toast,
        Self::Confirm,
        Self::Radar,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::Splash => "M1",
            Self::JobSpinner => "M2",
            Self::AgentPulse => "M3",
            Self::Progress => "M4",
            Self::Focus => "M5",
            Self::Drawer => "M6",
            Self::GraphExpand => "M7",
            Self::Toast => "M8",
            Self::Confirm => "M9",
            Self::Radar => "M10",
        }
    }
}

/// One row of the motion catalogue (purpose, frames, duration, reduced/off fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionRow {
    pub code: &'static str,
    pub kind: AnimKind,
    pub means: &'static str,
    pub frames: u16,
    pub ticks: u16,
    pub duration_ms: u16,
    pub easing: Easing,
    pub reduced: &'static str,
    pub off: &'static str,
    pub looping: bool,
}

/// The only animations the TUI may play. New motion needs a row here and in the UX plan.
pub const CATALOGUE: &[MotionRow] = &[
    MotionRow {
        code: "M1",
        kind: AnimKind::Splash,
        means: "the app is opening your case",
        frames: 10,
        ticks: 10,
        duration_ms: 600,
        easing: Easing::EaseOutCubic,
        reduced: "1 static frame, 150 ms",
        off: "none; straight to Case",
        looping: false,
    },
    MotionRow {
        code: "M2",
        kind: AnimKind::JobSpinner,
        means: "a passive collector is running",
        frames: 10,
        ticks: 10,
        duration_ms: 600,
        easing: Easing::Linear,
        reduced: "… 1 Hz",
        off: "running text",
        looping: true,
    },
    MotionRow {
        code: "M3",
        kind: AnimKind::AgentPulse,
        means: "the agent is thinking",
        frames: 4,
        ticks: 16,
        duration_ms: 960,
        easing: Easing::Step,
        reduced: "◆…",
        off: "thinking text",
        looping: true,
    },
    MotionRow {
        code: "M4",
        kind: AnimKind::Progress,
        means: "known-size work (CT pages, ingest rows, playbook steps)",
        frames: 12,
        ticks: 1,
        duration_ms: 0,
        easing: Easing::Linear,
        reduced: "updates ≤ 2 Hz",
        off: "42% text",
        looping: false,
    },
    MotionRow {
        code: "M5",
        kind: AnimKind::Focus,
        means: "focus moved to another pane",
        frames: 3,
        ticks: 3,
        duration_ms: 180,
        easing: Easing::Step,
        reduced: "instant",
        off: "instant",
        looping: false,
    },
    MotionRow {
        code: "M6",
        kind: AnimKind::Drawer,
        means: "the Inspector drawer opens (narrow terminals)",
        frames: 4,
        ticks: 4,
        duration_ms: 240,
        easing: Easing::EaseOutCubic,
        reduced: "instant",
        off: "instant",
        looping: false,
    },
    MotionRow {
        code: "M7",
        kind: AnimKind::GraphExpand,
        means: "Search Around / + added hops",
        frames: 6,
        ticks: 6,
        duration_ms: 360,
        easing: Easing::EaseOutCubic,
        reduced: "new nodes appear marked +, no reveal",
        off: "same as reduced",
        looping: false,
    },
    MotionRow {
        code: "M8",
        kind: AnimKind::Toast,
        means: "a job finished, failed or was refused",
        frames: 4,
        ticks: 4,
        duration_ms: 240,
        easing: Easing::EaseOutCubic,
        reduced: "appears in place, hold 4 s",
        off: "goes to the message line only",
        looping: false,
    },
    MotionRow {
        code: "M9",
        kind: AnimKind::Confirm,
        means: "A armed a write — no countdown, never auto-confirms",
        frames: 3,
        ticks: 3,
        duration_ms: 180,
        easing: Easing::EaseOutCubic,
        reduced: "instant, no pulse",
        off: "instant",
        looping: false,
    },
    MotionRow {
        code: "M10",
        kind: AnimKind::Radar,
        means: "passive collection is observing a target",
        frames: 8,
        ticks: 16,
        duration_ms: 960,
        easing: Easing::Linear,
        reduced: "static ◎ observing",
        off: "observing text",
        looping: true,
    },
];

pub fn catalogue_row(kind: AnimKind) -> &'static MotionRow {
    CATALOGUE
        .iter()
        .find(|row| row.kind == kind)
        .expect("every AnimKind has a catalogue row")
}

/// One live animation. Widgets key off [`Anim::id`].
#[derive(Clone, Debug, PartialEq)]
pub struct Anim {
    pub id: String,
    pub kind: AnimKind,
    pub start_tick: u64,
    pub frames: u16,
    pub easing: Easing,
    pub region: Rect,
    pub looping: bool,
    /// Ticks each visual frame occupies. Agent pulse is 4 ticks/frame; radar is 2.
    pub ticks_per_frame: u16,
}

/// Snapshot of one animation at a tick. Pure input to widget drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameView {
    pub index: u16,
    pub frames: u16,
    pub t: f64,
    pub eased: f64,
    pub looping: bool,
    pub done: bool,
}

impl FrameView {
    pub fn first(frames: u16) -> Self {
        Self {
            index: 0,
            frames,
            t: 0.0,
            eased: 0.0,
            looping: false,
            done: false,
        }
    }

    pub fn last(frames: u16) -> Self {
        let last = frames.saturating_sub(1);
        Self {
            index: last,
            frames,
            t: 1.0,
            eased: 1.0,
            looping: false,
            done: true,
        }
    }
}

/// Toast kind. Refusals always include a reason in [`Toast::message`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Ok,
    Err,
    Refused,
}

/// Slide-in, static hold, then ease-in-quad slide-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastPhase {
    In,
    Hold,
    Out,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
    pub phase: ToastPhase,
    pub age: u16,
    pub hold_until: Instant,
}

const TOAST_IN_FRAMES: u16 = 4;
const TOAST_OUT_FRAMES: u16 = 3;
const SPLASH_REDUCED: Duration = Duration::from_millis(150);
const SPINNER_REDUCED: Duration = Duration::from_secs(1);
const PROGRESS_REDUCED: Duration = Duration::from_millis(500);
const TOAST_HOLD_OK: Duration = Duration::from_secs(3);
const TOAST_HOLD_ERR: Duration = Duration::from_secs(8);
const TOAST_HOLD_REDUCED: Duration = Duration::from_secs(4);
const PROGRESS_FLASH: Duration = Duration::from_millis(60);
const PROGRESS_HOLD: Duration = Duration::from_millis(1500);

/// Why a tick ran. None of these write case data. Exhaustive: a new variant must opt out of writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickEffect {
    Advanced,
    SplashDone,
    ToastExpired,
    ConfirmPulseDone,
    GraphRevealDone,
    DrawerDone,
    FocusDone,
}

impl TickEffect {
    /// Ticks never accept, never save, never run a tool.
    pub fn writes(self) -> bool {
        match self {
            Self::Advanced
            | Self::SplashDone
            | Self::ToastExpired
            | Self::ConfirmPulseDone
            | Self::GraphRevealDone
            | Self::DrawerDone
            | Self::FocusDone => false,
        }
    }
}

/// What the event loop should wait on. Reduced never uses the 60 ms interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerNeed {
    None,
    Interval,
    Deadline(Instant),
}

#[derive(Clone, Debug, PartialEq)]
struct ProgressVisual {
    current: u32,
    total: u32,
    fill: ProgressFill,
    until: Option<Instant>,
}

fn is_transition(kind: AnimKind) -> bool {
    matches!(
        kind,
        AnimKind::Splash
            | AnimKind::Focus
            | AnimKind::Drawer
            | AnimKind::GraphExpand
            | AnimKind::Confirm
    )
}

/// Armed-bar visual phase. [`ConfirmPhase::Armed`] waits forever — no deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmPhase {
    Idle,
    Rising,
    Armed,
    Pulse,
    Dropping,
}

const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const AGENT_PULSE: [&str; 4] = ["◇", "◈", "◆", "◈"];
const AGENT_PULSE_ASCII: [&str; 4] = ["*", "+", "*", "+"];
const RADAR_ARROWS: [&str; 8] = ["↑", "↗", "→", "↘", "↓", "↙", "←", "↖"];
const RADAR_ARROWS_ASCII: [&str; 8] = ["^", "/", ">", "\\", "v", "/", "<", "\\"];
const EIGHTHS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
const DENSITY: [char; 4] = ['░', '▒', '▓', '█'];
const SPLASH_WORD: [&str; 5] = [" ", ".", "░░ ░  ░", "▒▓ ▒▓▒ ▒▓", "os-int"];

/// Holds live [`Anim`]s. Created empty; the event loop drops its 60 ms interval when this is idle.
#[derive(Clone, Debug)]
pub struct Animator {
    mode: MotionMode,
    configured: MotionMode,
    tick: u64,
    anims: Vec<Anim>,
    prev_index: HashMap<String, u16>,
    changed: bool,
    toasts: Vec<Toast>,
    toast_queue: VecDeque<Toast>,
    splash_case: String,
    splash_marking: String,
    confirm: ConfirmPhase,
    confirm_pulse_left: u16,
    graph_hops: u16,
    lag_behind: u32,
    last_tick_at: Option<Instant>,
    scanline: bool,
    ascii: bool,
    splash_until: Option<Instant>,
    spinner_until: Option<Instant>,
    progress_until: Option<Instant>,
    last_progress_at: Option<Instant>,
    progress_pending: bool,
    reduced_busy: bool,
    progress: HashMap<String, ProgressVisual>,
}

impl Animator {
    pub fn new(mode: MotionMode) -> Self {
        Self {
            mode,
            configured: mode,
            tick: 0,
            anims: Vec::new(),
            prev_index: HashMap::new(),
            changed: false,
            toasts: Vec::new(),
            toast_queue: VecDeque::new(),
            splash_case: String::new(),
            splash_marking: String::new(),
            confirm: ConfirmPhase::Idle,
            confirm_pulse_left: 0,
            graph_hops: 1,
            lag_behind: 0,
            last_tick_at: None,
            scanline: false,
            ascii: false,
            splash_until: None,
            spinner_until: None,
            progress_until: None,
            last_progress_at: None,
            progress_pending: false,
            reduced_busy: false,
            progress: HashMap::new(),
        }
    }

    pub fn from_config(config: &Config, env: &TerminalEnv, plain: bool) -> Self {
        let mode = resolve_motion(&config.ui.motion, env, plain);
        let mut animator = Self::new(mode);
        animator.scanline = config.ui.scanline;
        animator.ascii = config.ui.ascii || plain;
        animator
    }

    pub fn mode(&self) -> MotionMode {
        self.mode
    }

    pub fn configured_mode(&self) -> MotionMode {
        self.configured
    }

    pub fn set_ascii(&mut self, ascii: bool) {
        self.ascii = ascii;
    }

    pub fn ascii(&self) -> bool {
        self.ascii
    }

    pub fn scanline_enabled(&self) -> bool {
        self.scanline && self.mode == MotionMode::Full
    }

    pub fn set_scanline(&mut self, on: bool) {
        self.scanline = on;
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// True when the event loop must hold a 60 ms interval. Idle is zero timer wakeups.
    pub fn wants_timer(&self) -> bool {
        matches!(self.timer_need(), TimerNeed::Interval)
    }

    pub fn timer_need(&self) -> TimerNeed {
        if self.needs_interval() {
            return TimerNeed::Interval;
        }
        match self.next_deadline() {
            Some(at) => TimerNeed::Deadline(at),
            None => TimerNeed::None,
        }
    }

    fn needs_interval(&self) -> bool {
        if self.mode != MotionMode::Full {
            return false;
        }
        self.anims.iter().any(|anim| {
            matches!(
                anim.kind,
                AnimKind::Splash
                    | AnimKind::JobSpinner
                    | AnimKind::AgentPulse
                    | AnimKind::Focus
                    | AnimKind::Drawer
                    | AnimKind::GraphExpand
                    | AnimKind::Toast
                    | AnimKind::Confirm
                    | AnimKind::Radar
            )
        }) || self
            .toasts
            .iter()
            .any(|toast| matches!(toast.phase, ToastPhase::In | ToastPhase::Out))
            || matches!(
                self.confirm,
                ConfirmPhase::Rising | ConfirmPhase::Pulse | ConfirmPhase::Dropping
            )
    }

    fn next_deadline(&self) -> Option<Instant> {
        let mut soonest: Option<Instant> = None;
        let mut consider = |candidate: Option<Instant>| {
            if let Some(at) = candidate {
                soonest = Some(match soonest {
                    Some(current) => current.min(at),
                    None => at,
                });
            }
        };
        consider(self.splash_until);
        if self.reduced_busy {
            consider(self.spinner_until);
        }
        if self.progress_pending {
            consider(self.progress_until);
        }
        for visual in self.progress.values() {
            consider(visual.until);
        }
        for toast in &self.toasts {
            if toast.phase == ToastPhase::Hold {
                consider(Some(toast.hold_until));
            }
        }
        soonest
    }

    pub fn has_active(&self) -> bool {
        !matches!(self.timer_need(), TimerNeed::None)
    }

    /// Interval is live only while this is true. Tests use this as the idle=0 proof.
    pub fn interval_live(&self) -> bool {
        self.wants_timer()
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.timer_need(), TimerNeed::None)
    }

    pub fn freeze_at(&self, tick: u64) -> Self {
        let mut frozen = self.clone();
        frozen.tick = tick;
        for toast in &mut frozen.toasts {
            let age = tick.min(u64::from(u16::MAX)) as u16;
            if age < TOAST_IN_FRAMES && toast.phase == ToastPhase::In {
                toast.age = age;
            } else {
                toast.phase = ToastPhase::Hold;
                toast.age = age.saturating_sub(TOAST_IN_FRAMES);
            }
        }
        frozen.recompute_changed();
        frozen
    }

    pub fn frame(&self, id: &str) -> Option<FrameView> {
        let anim = self.anims.iter().find(|anim| anim.id == id)?;
        Some(self.view_of(anim))
    }

    pub fn frame_kind(&self, kind: AnimKind) -> Option<FrameView> {
        let anim = self.anims.iter().find(|anim| anim.kind == kind)?;
        Some(self.view_of(anim))
    }

    pub fn playing(&self, kind: AnimKind) -> bool {
        self.anims.iter().any(|anim| anim.kind == kind)
    }

    /// Advance one 60 ms tick or a one-shot deadline. Never accepts a Review item.
    pub fn advance(&mut self) -> Vec<TickEffect> {
        let mut effects = Vec::new();
        if self.mode == MotionMode::Off {
            return effects;
        }
        self.changed = false;
        if self.mode == MotionMode::Full {
            self.note_wall_clock();
            self.tick = self.tick.saturating_add(1);
            let mut finished = Vec::new();
            for anim in &self.anims {
                let view = self.view_of(anim);
                let prev = self.prev_index.get(&anim.id).copied();
                if prev != Some(view.index) {
                    self.changed = true;
                }
                self.prev_index.insert(anim.id.clone(), view.index);
                if view.done && !anim.looping {
                    finished.push(anim.id.clone());
                    effects.push(match anim.kind {
                        AnimKind::Splash => TickEffect::SplashDone,
                        AnimKind::Confirm => TickEffect::ConfirmPulseDone,
                        AnimKind::GraphExpand => TickEffect::GraphRevealDone,
                        AnimKind::Drawer => TickEffect::DrawerDone,
                        AnimKind::Focus => TickEffect::FocusDone,
                        _ => TickEffect::Advanced,
                    });
                }
            }
            for id in finished {
                self.anims.retain(|anim| anim.id != id);
                self.prev_index.remove(&id);
            }
            self.advance_confirm(&mut effects);
        }
        self.advance_deadlines(&mut effects);
        self.advance_toasts(&mut effects);
        if self.changed {
            effects.push(TickEffect::Advanced);
        }
        debug_assert!(effects.iter().all(|effect| !effect.writes()));
        effects
    }

    fn note_wall_clock(&mut self) {
        let now = Instant::now();
        if let Some(previous) = self.last_tick_at {
            let behind = now.saturating_duration_since(previous).as_millis() as u64 / TICK_MS;
            if behind > 1 {
                self.lag_behind = self.lag_behind.saturating_add(behind as u32 - 1);
            } else {
                self.lag_behind = 0;
            }
            if self.lag_behind > LAG_TICKS_BEFORE_REDUCED && self.mode == MotionMode::Full {
                self.mode = MotionMode::Reduced;
                self.strip_travel();
            }
        }
        self.last_tick_at = Some(now);
    }

    fn strip_travel(&mut self) {
        self.anims.retain(|anim| {
            !matches!(
                anim.kind,
                AnimKind::Drawer | AnimKind::Toast | AnimKind::Radar | AnimKind::Splash
            )
        });
        for toast in &mut self.toasts {
            toast.phase = ToastPhase::Hold;
            toast.age = 0;
        }
    }

    fn advance_deadlines(&mut self, effects: &mut Vec<TickEffect>) {
        let now = Instant::now();
        if let Some(until) = self.splash_until
            && now >= until
        {
            self.splash_until = None;
            self.stop_kind(AnimKind::Splash);
            effects.push(TickEffect::SplashDone);
            self.changed = true;
        }
        if self.reduced_busy && self.spinner_until.is_some_and(|until| now >= until) {
            self.spinner_until = Some(now + SPINNER_REDUCED);
            self.changed = true;
        }
        if self.progress_pending && self.progress_until.is_some_and(|until| now >= until) {
            self.progress_pending = false;
            self.last_progress_at = Some(now);
            self.progress_until = None;
            self.changed = true;
        }
        let mut drop_keys = Vec::new();
        for (key, visual) in &mut self.progress {
            let Some(until) = visual.until else {
                continue;
            };
            if now < until {
                continue;
            }
            match visual.fill {
                ProgressFill::CompleteFlash => {
                    visual.fill = ProgressFill::Complete;
                    visual.until = Some(now + PROGRESS_HOLD);
                    self.changed = true;
                }
                ProgressFill::Complete | ProgressFill::Failed => {
                    drop_keys.push(key.clone());
                    self.changed = true;
                }
                ProgressFill::Running => visual.until = None,
            }
        }
        for key in drop_keys {
            self.progress.remove(&key);
        }
    }

    fn advance_toasts(&mut self, effects: &mut Vec<TickEffect>) {
        let now = Instant::now();
        let travel = self.mode.allows_travel();
        for toast in &mut self.toasts {
            match toast.phase {
                ToastPhase::In => {
                    toast.age = toast.age.saturating_add(1);
                    if toast.age >= TOAST_IN_FRAMES {
                        toast.phase = ToastPhase::Hold;
                        toast.age = 0;
                        toast.hold_until = Instant::now()
                            + match toast.kind {
                                ToastKind::Err | ToastKind::Refused => TOAST_HOLD_ERR,
                                _ => TOAST_HOLD_OK,
                            };
                    }
                }
                ToastPhase::Hold if now >= toast.hold_until => {
                    toast.phase = ToastPhase::Out;
                    toast.age = if travel { 0 } else { TOAST_OUT_FRAMES };
                }
                ToastPhase::Hold => {}
                ToastPhase::Out if travel => {
                    toast.age = toast.age.saturating_add(1);
                }
                ToastPhase::Out => toast.age = TOAST_OUT_FRAMES,
            }
        }
        let before = self.toasts.len();
        self.toasts
            .retain(|toast| !(toast.phase == ToastPhase::Out && toast.age >= TOAST_OUT_FRAMES));
        if self.toasts.len() != before {
            effects.push(TickEffect::ToastExpired);
            self.changed = true;
        }
        let sliding = self
            .toasts
            .iter()
            .any(|toast| matches!(toast.phase, ToastPhase::In | ToastPhase::Out));
        if travel
            && self
                .toasts
                .iter()
                .any(|toast| toast.phase == ToastPhase::Out)
        {
            if !self.playing(AnimKind::Toast) {
                self.play(AnimKind::Toast, "toast-out", Rect::new(0, 0, TOAST_COLS, 1));
                if let Some(anim) = self.anims.iter_mut().find(|anim| anim.id == "toast-out") {
                    anim.frames = TOAST_OUT_FRAMES;
                    anim.easing = Easing::EaseInQuad;
                }
            }
        } else if !sliding {
            self.stop_kind(AnimKind::Toast);
        }
        self.promote_toasts();
    }

    fn advance_confirm(&mut self, effects: &mut Vec<TickEffect>) {
        match self.confirm {
            ConfirmPhase::Rising if !self.playing(AnimKind::Confirm) => {
                self.confirm = ConfirmPhase::Armed;
                self.changed = true;
            }
            ConfirmPhase::Pulse => {
                if self.confirm_pulse_left > 0 {
                    self.confirm_pulse_left -= 1;
                    self.changed = true;
                }
                if self.confirm_pulse_left == 0 {
                    self.confirm = ConfirmPhase::Idle;
                    effects.push(TickEffect::ConfirmPulseDone);
                }
            }
            ConfirmPhase::Dropping if !self.playing(AnimKind::Confirm) => {
                self.confirm = ConfirmPhase::Idle;
                self.changed = true;
            }
            ConfirmPhase::Rising
            | ConfirmPhase::Dropping
            | ConfirmPhase::Armed
            | ConfirmPhase::Idle => {}
        }
    }

    fn view_of(&self, anim: &Anim) -> FrameView {
        if self.mode == MotionMode::Off {
            return FrameView::last(anim.frames);
        }
        let elapsed = self.tick.saturating_sub(anim.start_tick);
        let span = u64::from(anim.frames.max(1)) * u64::from(anim.ticks_per_frame.max(1));
        if anim.looping {
            let index = if span == 0 {
                0
            } else {
                ((elapsed / u64::from(anim.ticks_per_frame.max(1))) % u64::from(anim.frames)) as u16
            };
            let t = if anim.frames <= 1 {
                0.0
            } else {
                f64::from(index) / f64::from(anim.frames.saturating_sub(1))
            };
            return FrameView {
                index,
                frames: anim.frames,
                t,
                eased: anim.easing.apply(t),
                looping: true,
                done: false,
            };
        }
        if self.mode == MotionMode::Reduced && matches!(anim.kind, AnimKind::Splash) {
            return FrameView::last(anim.frames);
        }
        if self.mode == MotionMode::Reduced
            && matches!(
                anim.kind,
                AnimKind::Drawer | AnimKind::Focus | AnimKind::GraphExpand | AnimKind::Confirm
            )
        {
            return FrameView::last(anim.frames);
        }
        let raw_index = if anim.ticks_per_frame == 0 {
            elapsed as u16
        } else {
            (elapsed / u64::from(anim.ticks_per_frame)) as u16
        };
        let done = elapsed >= span;
        let index = raw_index.min(anim.frames.saturating_sub(1));
        let t = if anim.frames <= 1 {
            1.0
        } else {
            f64::from(index) / f64::from(anim.frames.saturating_sub(1))
        };
        FrameView {
            index,
            frames: anim.frames,
            t,
            eased: anim.easing.apply(t),
            looping: false,
            done,
        }
    }

    fn recompute_changed(&mut self) {
        self.changed = false;
        for anim in &self.anims {
            let view = self.view_of(anim);
            if self.prev_index.get(&anim.id).copied() != Some(view.index) {
                self.changed = true;
            }
        }
    }

    /// Skip `terminal.draw` on a tick unless some region produced a new frame index.
    pub fn needs_redraw(&self) -> bool {
        self.changed
    }

    pub fn play(&mut self, kind: AnimKind, id: impl Into<String>, region: Rect) {
        if self.mode == MotionMode::Off && !matches!(kind, AnimKind::Progress) {
            return;
        }
        if matches!(kind, AnimKind::Radar) && self.playing(AnimKind::Radar) {
            return;
        }
        if is_transition(kind) {
            self.anims.retain(|anim| !is_transition(anim.kind));
        }
        let row = catalogue_row(kind);
        let id = id.into();
        self.anims.retain(|anim| anim.id != id);
        let ticks_per_frame = match kind {
            AnimKind::AgentPulse => 4,
            AnimKind::Radar => 2,
            AnimKind::GraphExpand => 1,
            _ => 1,
        };
        self.anims.push(Anim {
            id,
            kind,
            start_tick: self.tick,
            frames: row.frames,
            easing: row.easing,
            region,
            looping: row.looping,
            ticks_per_frame,
        });
        self.changed = true;
    }

    pub fn stop_kind(&mut self, kind: AnimKind) {
        self.anims.retain(|anim| anim.kind != kind);
    }

    pub fn stop_id(&mut self, id: &str) {
        self.anims.retain(|anim| anim.id != id);
        self.prev_index.remove(id);
    }

    pub fn stop_prefix(&mut self, prefix: &str) {
        self.anims.retain(|anim| !anim.id.starts_with(prefix));
    }

    pub fn clear_reduced_busy(&mut self) {
        self.reduced_busy = false;
        self.spinner_until = None;
    }

    pub fn start_splash(&mut self, case_name: &str, marking: &str) {
        if self.mode == MotionMode::Off {
            return;
        }
        self.splash_case = case_name.to_string();
        self.splash_marking = marking.to_string();
        if self.mode == MotionMode::Reduced {
            self.splash_until = Some(Instant::now() + SPLASH_REDUCED);
            self.changed = true;
            return;
        }
        self.play(AnimKind::Splash, "splash", Rect::new(0, 0, 80, 24));
    }

    pub fn skip_splash(&mut self) {
        self.stop_kind(AnimKind::Splash);
        self.splash_until = None;
        self.changed = true;
    }

    pub fn splash_case(&self) -> &str {
        &self.splash_case
    }

    pub fn splash_marking(&self) -> &str {
        &self.splash_marking
    }

    pub fn showing_splash(&self) -> bool {
        self.playing(AnimKind::Splash) || self.splash_until.is_some()
    }

    pub fn start_focus(&mut self) {
        if !self.mode.allows_travel() {
            return;
        }
        self.play(AnimKind::Focus, "focus", Rect::new(0, 0, 20, 1));
    }

    pub fn start_drawer(&mut self) {
        if !self.mode.allows_travel() {
            return;
        }
        self.play(AnimKind::Drawer, "drawer", Rect::new(0, 0, 80, 12));
    }

    /// Ease-out-cubic heights as fractions of the target drawer: 42%, 71%, 90%, 100%.
    pub fn drawer_height(&self, target: u16) -> u16 {
        let Some(view) = self.frame_kind(AnimKind::Drawer) else {
            return target;
        };
        let fractions = [0.42, 0.71, 0.90, 1.00];
        let frac = fractions.get(view.index as usize).copied().unwrap_or(1.0);
        ((f64::from(target) * frac).round() as u16).max(1)
    }

    pub fn start_graph_expand(&mut self, hops: u16) {
        self.graph_hops = hops.clamp(1, 3);
        if !self.mode.allows_travel() {
            return;
        }
        let frames = (self.graph_hops * 2).min(6);
        self.play(AnimKind::GraphExpand, "graph", Rect::new(0, 0, 40, 12));
        if let Some(anim) = self
            .anims
            .iter_mut()
            .find(|anim| anim.kind == AnimKind::GraphExpand)
        {
            anim.frames = frames;
        }
    }

    /// Hop rings fully revealed. Reduced/off show everything immediately.
    pub fn graph_revealed_hop(&self) -> u16 {
        if !self.mode.allows_travel() {
            return self.graph_hops;
        }
        match self.frame_kind(AnimKind::GraphExpand) {
            Some(view) => ((view.index / 2) + 1).min(self.graph_hops),
            None => self.graph_hops,
        }
    }

    pub fn graph_edges_only(&self) -> bool {
        self.frame_kind(AnimKind::GraphExpand)
            .is_some_and(|view| view.index % 2 == 0 && !view.done && self.mode.allows_travel())
    }

    pub fn arm_confirm(&mut self) {
        match self.mode {
            MotionMode::Off | MotionMode::Reduced => {
                self.confirm = ConfirmPhase::Armed;
            }
            MotionMode::Full => {
                self.confirm = ConfirmPhase::Rising;
                self.play(AnimKind::Confirm, "confirm", Rect::new(0, 0, 80, 1));
            }
        }
        self.changed = true;
    }

    pub fn disarm_confirm(&mut self) {
        self.stop_kind(AnimKind::Confirm);
        if self.mode.allows_travel() && self.confirm != ConfirmPhase::Idle {
            self.confirm = ConfirmPhase::Dropping;
            self.play(AnimKind::Confirm, "confirm-out", Rect::new(0, 0, 80, 1));
            if let Some(anim) = self.anims.iter_mut().find(|anim| anim.id == "confirm-out") {
                anim.frames = 2;
                anim.easing = Easing::EaseInQuad;
            }
        } else {
            self.confirm = ConfirmPhase::Idle;
            self.stop_kind(AnimKind::Confirm);
        }
        self.changed = true;
    }

    /// Visual flash after `y`. The write already happened from the key — not from this timer.
    pub fn pulse_confirm(&mut self) {
        self.confirm = ConfirmPhase::Pulse;
        self.confirm_pulse_left = 2;
        self.stop_kind(AnimKind::Confirm);
        self.changed = true;
    }

    pub fn confirm_phase(&self) -> ConfirmPhase {
        self.confirm
    }

    pub fn confirm_rise_frame(&self) -> u16 {
        match self.confirm {
            ConfirmPhase::Idle => 2,
            ConfirmPhase::Rising => self.frame("confirm").map(|view| view.index).unwrap_or(0),
            ConfirmPhase::Armed | ConfirmPhase::Pulse => 2,
            ConfirmPhase::Dropping => self
                .frame("confirm-out")
                .map(|view| 1 - view.index.min(1))
                .unwrap_or(0),
        }
    }

    pub fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        if self.mode == MotionMode::Off {
            return;
        }
        let hold = match (self.mode, kind) {
            (_, ToastKind::Err | ToastKind::Refused) => TOAST_HOLD_ERR,
            (MotionMode::Reduced, _) => TOAST_HOLD_REDUCED,
            _ => TOAST_HOLD_OK,
        };
        let travel = self.mode.allows_travel();
        let toast = Toast {
            kind,
            message: message.into(),
            phase: if travel {
                ToastPhase::In
            } else {
                ToastPhase::Hold
            },
            age: 0,
            hold_until: Instant::now() + hold,
        };
        self.toast_queue.push_back(toast);
        self.promote_toasts();
        self.changed = true;
    }

    fn promote_toasts(&mut self) {
        while self.toasts.len() < TOAST_STACK
            && let Some(mut toast) = self.toast_queue.pop_front()
        {
            let sliding_in = self.toasts.iter().any(|item| item.phase == ToastPhase::In);
            if self.mode.allows_travel() && sliding_in {
                toast.phase = ToastPhase::Hold;
            }
            if toast.phase == ToastPhase::In {
                self.play(
                    AnimKind::Toast,
                    format!("toast-{}", self.toasts.len()),
                    Rect::new(0, 0, TOAST_COLS, 1),
                );
            }
            self.toasts.push(toast);
        }
    }

    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    pub fn collapsed_toast_count(&self) -> usize {
        self.toast_queue.len()
    }

    /// Dismiss err/refused toasts. Returns true when something was dismissed.
    pub fn dismiss_failed_toasts(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts
            .retain(|toast| !matches!(toast.kind, ToastKind::Err | ToastKind::Refused));
        let dropped = self.toasts.len() != before;
        if dropped {
            self.changed = true;
        }
        dropped
    }

    pub fn ensure_spinner(&mut self, job_key: impl Into<String>) {
        self.sync_spinner(&job_key.into(), Rect::new(0, 0, 1, 1));
    }

    pub fn sync_spinner(&mut self, job_key: &str, region: Rect) {
        match self.mode {
            MotionMode::Off => {}
            MotionMode::Reduced => {
                self.reduced_busy = true;
                if self.spinner_until.is_none() {
                    self.spinner_until = Some(Instant::now() + SPINNER_REDUCED);
                }
            }
            MotionMode::Full => {
                if self.frame(job_key).is_none() {
                    self.play(AnimKind::JobSpinner, job_key, region);
                }
            }
        }
    }

    pub fn sync_agent(&mut self, running: bool) {
        match (running, self.mode) {
            (true, MotionMode::Full) => {
                if !self.playing(AnimKind::AgentPulse) {
                    self.play(AnimKind::AgentPulse, "agent", Rect::new(0, 0, 2, 1));
                }
            }
            (true, MotionMode::Reduced) => {
                self.reduced_busy = true;
                if self.spinner_until.is_none() {
                    self.spinner_until = Some(Instant::now() + SPINNER_REDUCED);
                }
            }
            _ => self.stop_kind(AnimKind::AgentPulse),
        }
    }

    pub fn sync_radar(&mut self, observing: bool) {
        if observing && self.mode == MotionMode::Full {
            if !self.playing(AnimKind::Radar) {
                self.play(AnimKind::Radar, "radar", Rect::new(0, 0, 5, 3));
            }
        } else {
            self.stop_kind(AnimKind::Radar);
        }
    }

    pub fn sync_progress(&mut self, job_key: &str, known: bool) {
        if !known || self.mode == MotionMode::Off {
            self.progress.remove(job_key);
            self.stop_id(job_key);
        }
    }

    pub fn update_progress(&mut self, job_key: &str, current: u32, total: u32) {
        if self.mode == MotionMode::Off {
            return;
        }
        let entry = self
            .progress
            .entry(job_key.to_string())
            .or_insert(ProgressVisual {
                current,
                total,
                fill: ProgressFill::Running,
                until: None,
            });
        entry.current = current;
        entry.total = total.max(1);
        if !matches!(
            entry.fill,
            ProgressFill::CompleteFlash | ProgressFill::Complete | ProgressFill::Failed
        ) {
            entry.fill = ProgressFill::Running;
        }
    }

    pub fn complete_progress(&mut self, job_key: &str, current: u32, total: u32) {
        if self.mode == MotionMode::Off {
            return;
        }
        self.progress.insert(
            job_key.to_string(),
            ProgressVisual {
                current: current.min(total.max(1)),
                total: total.max(1),
                fill: ProgressFill::CompleteFlash,
                until: Some(Instant::now() + PROGRESS_FLASH),
            },
        );
        self.changed = true;
    }

    pub fn fail_progress(&mut self, job_key: &str, current: u32, total: u32) {
        if self.mode == MotionMode::Off {
            return;
        }
        self.progress.insert(
            job_key.to_string(),
            ProgressVisual {
                current,
                total: total.max(1),
                fill: ProgressFill::Failed,
                until: Some(Instant::now() + TOAST_HOLD_ERR),
            },
        );
        self.changed = true;
    }

    /// Reduced mode redraws known-size progress at most twice a second.
    pub fn allow_progress_redraw(&mut self) -> bool {
        if self.mode != MotionMode::Reduced {
            return true;
        }
        let now = Instant::now();
        if let Some(last) = self.last_progress_at
            && now.saturating_duration_since(last) < PROGRESS_REDUCED
        {
            self.progress_pending = true;
            self.progress_until = Some(last + PROGRESS_REDUCED);
            return false;
        }
        self.last_progress_at = Some(now);
        self.progress_pending = false;
        self.progress_until = None;
        true
    }

    pub fn has_progress_visuals(&self) -> bool {
        !self.progress.is_empty()
    }

    pub fn progress_visual(&self, job_key: &str) -> Option<(u32, u32, ProgressFill)> {
        self.progress
            .get(job_key)
            .map(|visual| (visual.current, visual.total, visual.fill))
    }

    pub fn progress_entries(&self) -> Vec<(String, u32, u32, ProgressFill)> {
        self.progress
            .iter()
            .map(|(key, visual)| (key.clone(), visual.current, visual.total, visual.fill))
            .collect()
    }

    pub fn job_spinner_glyph(&self, glyphs: &Glyphs) -> &'static str {
        match self.mode {
            MotionMode::Off => "running",
            MotionMode::Reduced => "…",
            MotionMode::Full => {
                let index = self
                    .frame_kind(AnimKind::JobSpinner)
                    .map(|view| view.index as usize)
                    .unwrap_or(0);
                if glyphs.ascii {
                    glyphs.spinner[index % glyphs.spinner.len()]
                } else {
                    BRAILLE[index % BRAILLE.len()]
                }
            }
        }
    }

    pub fn agent_glyph(&self, glyphs: &Glyphs) -> &'static str {
        match self.mode {
            MotionMode::Off => "thinking",
            MotionMode::Reduced => {
                if glyphs.ascii {
                    "*..."
                } else {
                    "◆…"
                }
            }
            MotionMode::Full => {
                let index = self
                    .frame_kind(AnimKind::AgentPulse)
                    .map(|view| view.index as usize)
                    .unwrap_or(0);
                if glyphs.ascii {
                    AGENT_PULSE_ASCII[index % AGENT_PULSE_ASCII.len()]
                } else {
                    AGENT_PULSE[index % AGENT_PULSE.len()]
                }
            }
        }
    }

    pub fn agent_style(&self, theme: &Theme) -> Style {
        let base = theme.ai;
        if self.mode != MotionMode::Full || theme.depth != ColorDepth::TrueColor {
            return base;
        }
        let index = self
            .frame_kind(AnimKind::AgentPulse)
            .map(|view| view.index)
            .unwrap_or(2);
        // 40% → 70% → 100% → 70%
        let factor = match index {
            0 => 0.40,
            1 | 3 => 0.70,
            _ => 1.00,
        };
        theme.scale_fg(Role::Ai, factor)
    }

    pub fn radar_arrow(&self, glyphs: &Glyphs) -> &'static str {
        if self.mode != MotionMode::Full {
            return "";
        }
        let index = self
            .frame_kind(AnimKind::Radar)
            .map(|view| view.index as usize)
            .unwrap_or(0);
        if glyphs.ascii {
            RADAR_ARROWS_ASCII[index % RADAR_ARROWS_ASCII.len()]
        } else {
            RADAR_ARROWS[index % RADAR_ARROWS.len()]
        }
    }

    pub fn radar_index(&self) -> usize {
        self.frame_kind(AnimKind::Radar)
            .map(|view| view.index as usize)
            .unwrap_or(0)
    }

    pub fn toast_slide(&self, toast: &Toast) -> u16 {
        if !self.mode.allows_travel() {
            return 0;
        }
        match toast.phase {
            ToastPhase::Hold => 0,
            ToastPhase::In => match toast.age.min(3) {
                0 => TOAST_COLS.saturating_sub(2),
                1 => TOAST_COLS / 2,
                2 => TOAST_COLS / 6,
                _ => 0,
            },
            ToastPhase::Out => {
                let last = TOAST_OUT_FRAMES.saturating_sub(1).max(1);
                let t = f64::from(toast.age.min(last)) / f64::from(last);
                let eased = Easing::EaseInQuad.apply(t);
                (eased * f64::from(TOAST_COLS.saturating_sub(2))).round() as u16
            }
        }
    }
}

/// Force Off for `--plain`, screen-reader, `NO_COLOR`, and `TERM=dumb`.
pub fn resolve_motion(configured: &str, env: &TerminalEnv, plain: bool) -> MotionMode {
    let term = env
        .term
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase();
    let screen_reader = std::env::var_os("OS_INT_PLAIN").is_some();
    if plain || screen_reader || env.no_color || term == "dumb" || term == "unknown" {
        return MotionMode::Off;
    }
    MotionMode::parse(configured).unwrap_or(MotionMode::Full)
}

/// `n/m` in a progress line means the total is known. Unknown totals never get a fake bar.
pub fn parse_known_progress(text: &str) -> Option<(u32, u32)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'/' {
                let current: u32 = text[start..i].parse().ok()?;
                i += 1;
                let denom_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > denom_start {
                    let total: u32 = text[denom_start..i].parse().ok()?;
                    if total > 0 {
                        return Some((current.min(total), total));
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Source-count bands for graph-edge heat. Heat is corroboration, never risk.
pub fn corroboration_band(source_count: usize) -> u8 {
    match source_count {
        0 | 1 => 0,
        2 | 3 => 1,
        4..=6 => 2,
        _ => 3,
    }
}

pub fn corroboration_stroke(source_count: usize, ascii: bool) -> &'static str {
    if ascii {
        return match corroboration_band(source_count) {
            0 => "-",
            1 => "=",
            _ => "=",
        };
    }
    match corroboration_band(source_count) {
        0 => "─",
        1 => "━",
        _ => "═",
    }
}

pub fn splash_lines(
    view: FrameView,
    case: &str,
    marking: &str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let idx = view.index;
    let mut lines = vec![Line::from("")];
    let word = if idx <= 1 {
        SPLASH_WORD[1]
    } else if idx <= 3 {
        SPLASH_WORD[2]
    } else if idx <= 5 {
        SPLASH_WORD[3]
    } else {
        SPLASH_WORD[4]
    };
    lines.push(Line::from(Span::styled(format!("  {word}"), theme.fg)));
    if idx >= 6 {
        let filled = if idx == 6 { 3 } else { 6 };
        let rule: String = std::iter::repeat_n('─', filled).collect();
        let style = if idx == 6 { theme.heat0 } else { theme.heat1 };
        lines.push(Line::from(Span::styled(format!("  {rule}"), style)));
    } else {
        lines.push(Line::from(""));
    }
    if idx >= 8 {
        lines.push(Line::from(Span::styled(
            format!("  ▸ {case}  {marking}"),
            theme.muted,
        )));
        lines.push(Line::from(Span::styled(
            "  passive · ◆ proposes only",
            theme.accent,
        )));
    }
    lines
}

pub fn radar_lines(
    view: FrameView,
    theme: &Theme,
    ascii: bool,
    mode: MotionMode,
) -> Vec<Line<'static>> {
    if mode == MotionMode::Off {
        return vec![Line::from(Span::styled("observing", theme.muted))];
    }
    if mode == MotionMode::Reduced || ascii && mode != MotionMode::Full {
        let glyph = if ascii {
            "o observing"
        } else {
            "◎ observing"
        };
        return vec![Line::from(Span::styled(glyph, theme.muted))];
    }
    let index = view.index as usize % 8;
    let arrows = if ascii {
        RADAR_ARROWS_ASCII
    } else {
        RADAR_ARROWS
    };
    let core = if ascii { "O" } else { "◎" };
    // 5×3 compass: previous two positions stay at their cells (heat1, heat0).
    //  NW N  NE
    //  W  ◎  E
    //  SW S  SE
    let compass = [
        (2usize, 0usize), // N
        (4, 0),           // NE
        (4, 1),           // E
        (4, 2),           // SE
        (2, 2),           // S
        (0, 2),           // SW
        (0, 1),           // W
        (0, 0),           // NW
    ];
    let mut grid = vec![(" ".to_string(), theme.muted); 15];
    grid[7] = (core.to_string(), theme.muted);
    for (dir, style) in [
        (index, theme.heat2),
        ((index + 7) % 8, theme.heat1),
        ((index + 6) % 8, theme.heat0),
    ] {
        let (x, y) = compass[dir];
        grid[y * 5 + x] = (arrows[dir].to_string(), style);
    }
    grid.chunks(5)
        .map(|row| {
            Line::from(
                row.iter()
                    .map(|(ch, style)| Span::styled(ch.clone(), *style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

pub fn focus_style(view: Option<FrameView>, theme: &Theme) -> Style {
    match view.map(|frame| frame.index) {
        Some(0) => theme
            .accent
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED),
        Some(1) => theme.accent.add_modifier(Modifier::BOLD),
        _ => theme.accent,
    }
}

pub fn confirm_rise_line(frame: u16, text: &str, theme: &Theme, pulsing: bool) -> Line<'static> {
    let style = if pulsing { theme.ok } else { theme.fg };
    match frame {
        0 => Line::from(Span::styled("▁".repeat(32), theme.panel)),
        1 => Line::from(Span::styled("▄".repeat(32), theme.panel)),
        _ => Line::from(Span::styled(text.to_string(), style)).style(theme.panel),
    }
}

pub fn toast_line(toast: &Toast, theme: &Theme, glyphs: &Glyphs, pad: u16) -> Line<'static> {
    let (mark, style) = match toast.kind {
        ToastKind::Ok => (glyphs.done, theme.ok),
        ToastKind::Err => (glyphs.failed, theme.err),
        ToastKind::Refused => (glyphs.refused, theme.err),
    };
    let prefix: String = std::iter::repeat_n(' ', pad as usize).collect();
    Line::from(vec![
        Span::raw(prefix),
        Span::styled(format!("{mark} {}", toast.message), style),
    ])
}

/// How a known-size progress bar is filled. Unknown totals never get a bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressFill {
    Running,
    Failed,
    CompleteFlash,
    Complete,
}

pub fn progress_bar(
    current: u32,
    total: u32,
    theme: &Theme,
    mode: MotionMode,
    fill: ProgressFill,
) -> Line<'static> {
    let total = total.max(1);
    let ratio = f64::from(current.min(total)) / f64::from(total);
    let percent = (ratio * 100.0).round() as u32;
    if mode == MotionMode::Off {
        return Line::from(Span::styled(format!("{percent}%"), theme.muted));
    }
    let cells = 12usize;
    let active = matches!(fill, ProgressFill::Running);
    let failed = matches!(fill, ProgressFill::Failed);
    let complete_flash = matches!(fill, ProgressFill::CompleteFlash);
    let complete = matches!(fill, ProgressFill::Complete);
    let exact = ratio * cells as f64;
    let filled = exact.floor() as usize;
    let frac = ((exact - exact.floor()) * 8.0).round() as usize;
    let mut spans = Vec::new();
    spans.push(Span::raw("["));
    for i in 0..cells {
        let (ch, style) = if complete_flash {
            ('█', theme.ok)
        } else if complete {
            ('█', theme.muted)
        } else if failed && (i as f64) >= exact {
            ('▒', theme.err)
        } else if i < filled {
            let t = if cells == 1 {
                0.0
            } else {
                i as f64 / (cells as f64 - 1.0)
            };
            ('█', theme.heat_lerp(t))
        } else if i == filled && frac > 0 && !failed {
            let ch = EIGHTHS[(frac.saturating_sub(1)).min(7)];
            let style = if active { theme.heat2 } else { theme.heat1 };
            (ch, style)
        } else if theme.depth == ColorDepth::Ansi16 || theme.depth == ColorDepth::None {
            (DENSITY[0], theme.muted)
        } else {
            (' ', theme.muted)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans.push(Span::raw("]"));
    spans.push(Span::styled(format!(" {percent}%"), theme.muted));
    Line::from(spans)
}

pub fn progress_complete_flash(done: bool, tick_in_ok: bool) -> bool {
    done && tick_in_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UiConfig;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn theme() -> Theme {
        let mut config = Config::default();
        config.ui.theme = "dark".into();
        config.ui.color = "truecolor".into();
        Theme::resolve(
            &config,
            &TerminalEnv {
                colorterm: Some("truecolor".into()),
                term: Some("xterm-256color".into()),
                no_color: false,
                colorfgbg: None,
            },
        )
    }

    fn render_lines(lines: Vec<Line<'static>>, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                frame.render_widget(ratatui::widgets::Paragraph::new(lines), frame.area());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        let mut rows = Vec::new();
        for y in 0..area.height {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buffer[(x, y)].symbol());
            }
            rows.push(row.trim_end().to_string());
        }
        rows.join("\n")
    }

    fn golden(name: &str, actual: &str) -> Option<String> {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push(format!("tests/golden/motion/{name}.txt"));
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if expected.is_empty() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(&path, actual).expect("write");
            return Some(path.display().to_string());
        }
        assert_eq!(actual, expected, "snapshot mismatch for {name}\n{actual}");
        None
    }

    #[test]
    fn easing_curves_match_the_spec() {
        assert!((Easing::EaseOutCubic.apply(0.0) - 0.0).abs() < 1e-9);
        assert!((Easing::EaseOutCubic.apply(1.0) - 1.0).abs() < 1e-9);
        let mid = Easing::EaseOutCubic.apply(0.5);
        assert!((mid - (1.0 - 0.5_f64.powi(3))).abs() < 1e-9);
        assert!((Easing::EaseInQuad.apply(0.5) - 0.25).abs() < 1e-9);
        assert!((Easing::Linear.apply(0.3) - 0.3).abs() < 1e-9);
        assert!((Easing::Step.apply(0.8) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn idle_animator_holds_no_timer() {
        let animator = Animator::new(MotionMode::Full);
        assert!(animator.is_idle());
        assert!(!animator.wants_timer());
        assert!(!animator.interval_live());
        let off = Animator::new(MotionMode::Off);
        off.play_probe();
    }

    impl Animator {
        fn play_probe(&self) {
            assert!(!self.wants_timer());
        }
    }

    #[test]
    fn interval_is_dropped_when_the_last_animation_finishes() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.start_focus();
        assert!(animator.interval_live());
        for _ in 0..8 {
            animator.advance();
        }
        assert!(
            !animator.interval_live(),
            "idle must drop the 60 ms interval"
        );
        assert!(animator.is_idle());
    }

    #[test]
    fn tick_effects_never_write() {
        for effect in [
            TickEffect::Advanced,
            TickEffect::SplashDone,
            TickEffect::ToastExpired,
            TickEffect::ConfirmPulseDone,
            TickEffect::GraphRevealDone,
            TickEffect::DrawerDone,
            TickEffect::FocusDone,
        ] {
            assert!(!effect.writes(), "{effect:?} must not write");
        }
    }

    #[test]
    fn confirm_has_no_deadline_and_ticks_do_not_arm_or_accept() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.arm_confirm();
        assert_eq!(animator.confirm_phase(), ConfirmPhase::Rising);
        for _ in 0..400 {
            let effects = animator.advance();
            assert!(effects.iter().all(|effect| !effect.writes()));
            assert_ne!(animator.confirm_phase(), ConfirmPhase::Idle);
        }
        assert_eq!(animator.confirm_phase(), ConfirmPhase::Armed);
        assert!(
            !animator.playing(AnimKind::Confirm) || animator.confirm_phase() == ConfirmPhase::Armed
        );
    }

    #[test]
    fn every_animation_has_a_catalogue_row_and_a_plan_row() {
        let plan = include_str!("../../docs/UX_REVAMP_PLAN.md");
        for kind in AnimKind::ALL {
            let row = catalogue_row(kind);
            assert_eq!(row.kind, kind);
            assert_eq!(row.code, kind.code());
            assert!(
                plan.contains(&format!("| {} |", row.code)),
                "docs/UX_REVAMP_PLAN.md §8.2 must list {}",
                row.code
            );
        }
        assert_eq!(CATALOGUE.len(), AnimKind::ALL.len());
        let m9 = catalogue_row(AnimKind::Confirm);
        assert!(m9.means.contains("never auto-confirm") || m9.means.contains("no countdown"));
    }

    #[test]
    fn resolve_motion_forces_off_for_plain_nocolor_and_dumb() {
        let env = TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: false,
            colorfgbg: None,
        };
        assert_eq!(resolve_motion("full", &env, false), MotionMode::Full);
        assert_eq!(resolve_motion("full", &env, true), MotionMode::Off);
        let mut nocolor = env.clone();
        nocolor.no_color = true;
        assert_eq!(resolve_motion("full", &nocolor, false), MotionMode::Off);
        let mut dumb = env.clone();
        dumb.term = Some("dumb".into());
        assert_eq!(resolve_motion("reduced", &dumb, false), MotionMode::Off);
        assert_eq!(resolve_motion("off", &env, false), MotionMode::Off);
    }

    #[test]
    fn from_config_respects_ui_motion() {
        let config = Config {
            ui: UiConfig {
                motion: "reduced".into(),
                ..UiConfig::default()
            },
            ..Config::default()
        };
        let env = TerminalEnv::default();
        let animator = Animator::from_config(&config, &env, false);
        assert_eq!(animator.mode(), MotionMode::Reduced);
        assert_eq!(animator.configured_mode(), MotionMode::Reduced);
    }

    #[test]
    fn parse_known_progress_requires_a_total() {
        assert_eq!(parse_known_progress("crt.sh page 2/8"), Some((2, 8)));
        assert_eq!(parse_known_progress("42%"), None);
        assert_eq!(parse_known_progress("working"), None);
        assert_eq!(parse_known_progress("0/0"), None);
    }

    #[test]
    fn corroboration_heat_is_source_support_never_risk() {
        assert_eq!(corroboration_band(1), 0);
        assert_eq!(corroboration_band(3), 1);
        assert_eq!(corroboration_band(5), 2);
        assert_eq!(corroboration_band(9), 3);
        assert_eq!(corroboration_stroke(1, false), "─");
        assert_eq!(corroboration_stroke(3, false), "━");
        assert_eq!(corroboration_stroke(8, false), "═");
    }

    #[test]
    fn freeze_at_is_deterministic() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.play(AnimKind::JobSpinner, "j", Rect::new(0, 0, 1, 1));
        let a = animator.freeze_at(3).frame("j").expect("frame");
        let b = animator.freeze_at(3).frame("j").expect("frame");
        assert_eq!(a, b);
        assert_eq!(a.index, 3);
    }

    fn key_frames(kind: AnimKind) -> (u16, u16, u16) {
        let frames = catalogue_row(kind).frames;
        (0, frames / 2, frames.saturating_sub(1))
    }

    #[test]
    fn snapshots_first_mid_last_and_off_for_each_animation() {
        let theme = theme();
        let glyphs = Glyphs::unicode();
        let mut wrote = Vec::new();
        for kind in AnimKind::ALL {
            let (first, mid, last) = key_frames(kind);
            for (label, tick) in [("first", first), ("mid", mid), ("last", last)] {
                let mut animator = Animator::new(MotionMode::Full);
                match kind {
                    AnimKind::Splash => animator.start_splash("acme-phish", "TLP:AMBER"),
                    AnimKind::GraphExpand => animator.start_graph_expand(2),
                    AnimKind::Confirm => animator.arm_confirm(),
                    AnimKind::Drawer => animator.start_drawer(),
                    AnimKind::Focus => animator.start_focus(),
                    AnimKind::Toast => animator.push_toast(ToastKind::Ok, "ct done · 37 certs"),
                    _ => animator.play(kind, kind.code(), Rect::new(0, 0, 12, 3)),
                }
                let frozen = animator.freeze_at(tick.into());
                let text = widget_text(kind, &frozen, &theme, &glyphs, MotionMode::Full);
                wrote.extend(golden(
                    &format!("{}_{label}", kind.code().to_ascii_lowercase()),
                    &text,
                ));
            }
            let off = Animator::new(MotionMode::Off);
            let text = widget_text(kind, &off, &theme, &glyphs, MotionMode::Off);
            wrote.extend(golden(
                &format!("{}_off", kind.code().to_ascii_lowercase()),
                &text,
            ));
        }
        assert!(
            wrote.is_empty(),
            "wrote snapshots; re-run to confirm: {}",
            wrote.join(", ")
        );
    }

    fn widget_text(
        kind: AnimKind,
        animator: &Animator,
        theme: &Theme,
        glyphs: &Glyphs,
        mode: MotionMode,
    ) -> String {
        let view = animator
            .frame_kind(kind)
            .unwrap_or_else(|| FrameView::last(catalogue_row(kind).frames));
        let lines = match kind {
            AnimKind::Splash => splash_lines(view, "acme-phish", "TLP:AMBER", theme),
            AnimKind::JobSpinner => vec![Line::from(animator.job_spinner_glyph(glyphs))],
            AnimKind::AgentPulse => vec![Line::from(animator.agent_glyph(glyphs))],
            AnimKind::Progress => {
                vec![progress_bar(31, 100, theme, mode, ProgressFill::Running)]
            }
            AnimKind::Focus => vec![Line::from(Span::styled(
                format!("{}Inspector", glyphs.focus),
                focus_style(Some(view), theme),
            ))],
            AnimKind::Drawer => vec![Line::from(format!(
                "drawer {}%",
                animator.drawer_height(10) * 10
            ))],
            AnimKind::GraphExpand => vec![Line::from(format!(
                "hop {} edges_only={}",
                animator.graph_revealed_hop(),
                animator.graph_edges_only()
            ))],
            AnimKind::Toast => {
                let toast = animator.toasts().first().cloned().unwrap_or(Toast {
                    kind: ToastKind::Ok,
                    message: "ct done · 37 certs".into(),
                    phase: if mode.allows_travel() {
                        ToastPhase::In
                    } else {
                        ToastPhase::Hold
                    },
                    age: 0,
                    hold_until: Instant::now(),
                });
                if mode == MotionMode::Off {
                    vec![Line::from("message-line only")]
                } else {
                    vec![toast_line(
                        &toast,
                        theme,
                        glyphs,
                        animator.toast_slide(&toast),
                    )]
                }
            }
            AnimKind::Confirm => vec![confirm_rise_line(
                animator.confirm_rise_frame(),
                " Accept 1 AI finding? It keeps ◆ agent provenance.   y confirm · any other key cancels",
                theme,
                animator.confirm_phase() == ConfirmPhase::Pulse,
            )],
            AnimKind::Radar => radar_lines(view, theme, false, mode),
        };
        render_lines(lines, 48, 8)
    }

    #[test]
    fn reduced_and_off_have_no_travel() {
        for mode in [MotionMode::Reduced, MotionMode::Off] {
            let mut animator = Animator::new(mode);
            animator.start_drawer();
            animator.play(AnimKind::Radar, "radar", Rect::new(0, 0, 5, 3));
            animator.push_toast(ToastKind::Ok, "done");
            let theme = theme();
            let radar = render_lines(radar_lines(FrameView::first(8), &theme, false, mode), 24, 4);
            assert!(
                !radar.contains('↑') && !radar.contains('↗') && !radar.contains('→'),
                "sweep travel in {mode:?}: {radar}"
            );
            if mode == MotionMode::Off {
                assert!(animator.toasts().is_empty());
            }
            assert!(!animator.playing(AnimKind::Drawer) || !mode.allows_travel());
        }
    }

    #[test]
    fn splash_is_skippable() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.start_splash("acme-phish", "TLP:AMBER");
        assert!(animator.showing_splash());
        animator.skip_splash();
        assert!(!animator.showing_splash());
    }

    #[test]
    fn refusal_toast_states_why() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.push_toast(
            ToastKind::Refused,
            "active tool refused: nmap — os-int observes public information only",
        );
        let toast = animator.toasts().first().expect("toast");
        assert!(toast.message.contains("nmap"));
        assert!(toast.message.contains("refused"));
        assert!(
            toast.message.contains("observes")
                || toast.message.contains("why")
                || toast.message.contains("only")
        );
    }

    #[test]
    fn concurrency_caps_one_radar_and_queues_extra_toasts() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.sync_radar(true);
        animator.sync_radar(true);
        assert_eq!(
            animator
                .anims
                .iter()
                .filter(|anim| anim.kind == AnimKind::Radar)
                .count(),
            1
        );
        for i in 0..5 {
            animator.push_toast(ToastKind::Ok, format!("done {i}"));
        }
        assert!(animator.toasts().len() <= TOAST_STACK);
        assert!(animator.collapsed_toast_count() > 0 || animator.toasts().len() == TOAST_STACK);
    }

    #[test]
    fn progress_unknown_total_has_no_bar() {
        assert!(parse_known_progress("Querying DNS").is_none());
    }

    #[test]
    fn reduced_spinner_is_static_ellipsis_with_no_interval() {
        let mut animator = Animator::new(MotionMode::Reduced);
        animator.sync_spinner("j", Rect::new(0, 0, 1, 1));
        assert!(
            !animator.wants_timer(),
            "reduced must not hold a 60 ms timer"
        );
        assert!(matches!(animator.timer_need(), TimerNeed::Deadline(_)));
        assert_eq!(animator.job_spinner_glyph(&Glyphs::unicode()), "…");
        assert!(!animator.playing(AnimKind::JobSpinner));
    }

    #[test]
    fn reduced_splash_is_one_static_frame_for_150ms() {
        let mut animator = Animator::new(MotionMode::Reduced);
        let start = Instant::now();
        animator.start_splash("acme-phish", "TLP:AMBER");
        assert!(animator.showing_splash());
        assert!(!animator.wants_timer());
        match animator.timer_need() {
            TimerNeed::Deadline(at) => {
                let wait = at.saturating_duration_since(start);
                assert!(
                    wait <= SPLASH_REDUCED + Duration::from_millis(20),
                    "splash deadline {wait:?} exceeds 150 ms"
                );
                assert!(wait >= Duration::from_millis(100), "{wait:?}");
            }
            other => panic!("expected deadline, got {other:?}"),
        }
        let theme = theme();
        let text = render_lines(
            splash_lines(FrameView::last(10), "acme-phish", "TLP:AMBER", &theme),
            48,
            8,
        );
        assert!(text.contains("os-int"), "{text}");
        animator.skip_splash();
        assert!(!animator.showing_splash());
    }

    #[test]
    fn reduced_progress_throttles_to_2hz() {
        let mut animator = Animator::new(MotionMode::Reduced);
        assert!(animator.allow_progress_redraw());
        assert!(!animator.allow_progress_redraw());
        match animator.timer_need() {
            TimerNeed::Deadline(_) => {}
            other => panic!("pending progress needs a deadline, got {other:?}"),
        }
    }

    #[test]
    fn toast_hold_uses_a_deadline_not_an_interval() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.push_toast(ToastKind::Ok, "ct done · 37 certs");
        assert!(animator.wants_timer(), "slide-in needs 60 ms");
        for _ in 0..TOAST_IN_FRAMES {
            animator.advance();
        }
        let toast = animator.toasts().first().expect("toast");
        assert_eq!(toast.phase, ToastPhase::Hold);
        assert!(
            !animator.wants_timer(),
            "static hold must drop the 60 ms interval"
        );
        assert!(matches!(animator.timer_need(), TimerNeed::Deadline(_)));
    }

    #[test]
    fn toast_slide_out_is_three_ease_in_quad_frames() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.push_toast(ToastKind::Ok, "ct done · 37 certs");
        let mut toast = animator.toasts().first().cloned().expect("toast");
        toast.phase = ToastPhase::Out;
        toast.age = 0;
        let a = animator.toast_slide(&toast);
        toast.age = 1;
        let b = animator.toast_slide(&toast);
        toast.age = 2;
        let c = animator.toast_slide(&toast);
        assert!(a < b && b < c, "ease-in-quad pad {a} {b} {c}");
        assert_eq!(c, TOAST_COLS.saturating_sub(2));
    }

    #[test]
    fn concurrent_transitions_cap_at_one() {
        let mut animator = Animator::new(MotionMode::Full);
        animator.start_drawer();
        animator.start_focus();
        animator.start_graph_expand(2);
        let transitions = animator
            .anims
            .iter()
            .filter(|anim| is_transition(anim.kind))
            .count();
        assert_eq!(transitions, 1);
    }

    #[test]
    fn complete_flash_then_muted_and_failed_remainder() {
        let theme = theme();
        let flash = render_lines(
            vec![progress_bar(
                100,
                100,
                &theme,
                MotionMode::Full,
                ProgressFill::CompleteFlash,
            )],
            20,
            1,
        );
        let muted = render_lines(
            vec![progress_bar(
                100,
                100,
                &theme,
                MotionMode::Full,
                ProgressFill::Complete,
            )],
            20,
            1,
        );
        let failed = render_lines(
            vec![progress_bar(
                54,
                100,
                &theme,
                MotionMode::Full,
                ProgressFill::Failed,
            )],
            20,
            1,
        );
        assert!(flash.contains('█'), "{flash}");
        assert!(muted.contains('█'), "{muted}");
        assert!(failed.contains('▒'), "{failed}");
        let mut animator = Animator::new(MotionMode::Full);
        animator.complete_progress("prog:1", 8, 8);
        let visual = animator.progress_visual("prog:1").expect("flash");
        assert_eq!(visual.2, ProgressFill::CompleteFlash);
        animator.fail_progress("prog:2", 4, 8);
        let visual = animator.progress_visual("prog:2").expect("fail");
        assert_eq!(visual.2, ProgressFill::Failed);
    }
}
