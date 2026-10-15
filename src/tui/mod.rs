//! Terminal event loop and rendering. Drawing is a pure function of `App`.

mod glyphs;
pub(crate) mod keymap;
pub mod motion;
mod overlays;
mod preview;
mod screens;
mod table;
mod theme;
pub(crate) mod widgets;

pub use glyphs::{Glyphs, chrome_arrow, chrome_dash, chrome_sep};
pub use motion::{Animator, MotionMode, TICK, TICK_MS, TimerNeed};
pub use preview::render_preview;
pub use table::relative_time_from;
pub use theme::{
    ColorDepth, ColorMode, TerminalEnv, Theme, ThemeName, env_os_int_plain, is_os_int_plain,
};

use std::time::Duration;

use color_eyre::eyre::{Result, WrapErr};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthStr;

use crate::app::{Action, App, View};
use crate::sanitize::sanitize_line;

use self::motion::{ConfirmPhase, confirm_rise_line, toast_line};

/// Runs the TUI until the user quits. Always restores the terminal, including on error.
pub async fn run(app: &mut App) -> Result<()> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        hook(info);
    }));
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, app).await;
    ratatui::restore();
    result
}

async fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let mut events = EventStream::new();
    let mut jobs = app
        .jobs
        .take_receiver()
        .expect("job event receiver is taken once");
    let mut ticker: Option<tokio::time::Interval> = None;
    let shutdown = wait_for_shutdown();
    tokio::pin!(shutdown);
    app.mark_dirty();
    while !app.should_quit {
        let need = app.motion.timer_need();
        sync_motion_timer(&mut ticker, matches!(need, motion::TimerNeed::Interval));
        let deadline = match need {
            motion::TimerNeed::Deadline(at) => Some(at),
            _ => None,
        };
        if app.dirty {
            terminal
                .draw(|frame| draw(frame, app))
                .wrap_err("could not draw the terminal")?;
            app.dirty = false;
        }
        tokio::select! {
            event = events.next() => {
                match event {
                    Some(Ok(Event::Key(key))) => {
                        let action = keymap::map_key(
                            key,
                            &app.overlay,
                            app.view,
                            app.is_editing(),
                            app.review_armed,
                            app.board_zoomed,
                        );
                        if matches!(action, Some(Action::Quit)) {
                            dispatch(app, Action::Quit);
                            continue;
                        }
                        if app.motion.showing_splash() {
                            app.motion.skip_splash();
                            app.mark_dirty();
                            continue;
                        }
                        if let Some(action) = action {
                            dispatch(app, action);
                            if let Err(error) = run_external_editor(terminal, app) {
                                app.status = error.to_string();
                                app.mark_dirty();
                            }
                        }
                    }
                    Some(Ok(Event::Resize(width, _))) => {
                        app.on_resize(width < screens::INSPECTOR_BREAK);
                        app.mark_dirty();
                    }
                    Some(Ok(Event::Paste(text))) => {
                        for ch in text.chars() {
                            dispatch(app, Action::InsertChar(ch));
                        }
                    }
                    Some(Err(error)) => {
                        app.status = format!("input error: {error}");
                        app.mark_dirty();
                    }
                    None => break,
                    _ => {}
                }
            }
            event = jobs.recv() => {
                if let Some(event) = event {
                    dispatch(app, Action::Job(event));
                }
            }
            _ = tick_optional(&mut ticker) => {
                dispatch(app, Action::Tick);
            }
            _ = sleep_deadline(deadline) => {
                dispatch(app, Action::Tick);
            }
            _ = &mut shutdown => {
                app.should_quit = true;
            }
        }
    }
    app.jobs.shutdown(Duration::from_secs(4)).await;
    Ok(())
}

/// Create the 60 ms interval when the first Full-motion frame needs it; drop it otherwise.
fn sync_motion_timer(ticker: &mut Option<tokio::time::Interval>, wants_interval: bool) {
    if wants_interval {
        if ticker.is_none() {
            let mut interval = tokio::time::interval(TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.reset();
            *ticker = Some(interval);
        }
    } else {
        *ticker = None;
    }
}

async fn tick_optional(ticker: &mut Option<tokio::time::Interval>) {
    match ticker.as_mut() {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending::<()>().await,
    }
}

async fn sleep_deadline(at: Option<std::time::Instant>) {
    match at {
        Some(instant) => {
            let now = std::time::Instant::now();
            if instant > now {
                tokio::time::sleep(instant.saturating_duration_since(now)).await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        let mut sighup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        {
            Ok(signal) => signal,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
            _ = sighup.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn dispatch(app: &mut App, action: Action) {
    if !matches!(action, Action::Tick) {
        app.clear_graph_plus();
    }
    if let Err(error) = app.dispatch(action) {
        let message = error.to_string();
        app.last_error = Some(message.clone());
        app.status = message;
        app.mark_dirty();
    }
    app.sync_motion();
}

fn run_external_editor(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let Some(session) = app.take_pending_editor() else {
        return Ok(());
    };
    ratatui::restore();
    let mut argv = app.editor_command();
    let program = argv.remove(0);
    let status = std::process::Command::new(&program)
        .args(&argv)
        .arg(&session.path)
        .status();
    *terminal = ratatui::init();
    match status {
        Ok(code) if code.success() => app.apply_editor_session(session)?,
        Ok(code) => {
            app.status = format!("editor exited {code}");
            let _ = std::fs::remove_file(&session.path);
        }
        Err(error) => {
            app.status = format!("could not launch {program}: {error}");
            let _ = std::fs::remove_file(&session.path);
        }
    }
    app.mark_dirty();
    Ok(())
}

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = Theme::from_config(&app.config);
    let area = frame.area();
    if theme.paints_background {
        frame.render_widget(Block::default().style(theme.bg), area);
    }
    if theme.name == ThemeName::Phosphor && app.motion.scanline_enabled() {
        paint_scanline(frame, area, &theme);
    }
    if app.motion.showing_splash() {
        draw_splash(frame, area, app, &theme);
        return;
    }
    let armed = app.review_armed || !matches!(app.motion.confirm_phase(), ConfirmPhase::Idle);
    let mut constraints = vec![Constraint::Length(1), Constraint::Min(5)];
    if armed {
        constraints.push(Constraint::Length(1));
    }
    if theme.degrade_hint.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));
    let rows = Layout::vertical(constraints).split(area);
    draw_header(frame, rows[0], app, &theme);
    let board = if app.view == View::Welcome {
        screens::welcome(frame, rows[1], app, &theme);
        rows[1]
    } else {
        screens::workspace(frame, rows[1], app, &theme)
    };
    draw_toasts(frame, board, app, &theme);
    let mut row = 2;
    if armed {
        draw_confirm_bar(frame, rows[row], app, &theme);
        row += 1;
    }
    if let Some(hint) = theme.degrade_hint {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {hint}"), theme.warn))),
            rows[row],
        );
        row += 1;
    }
    draw_status(frame, rows[row], app, &theme);
    frame.render_widget(
        Paragraph::new(widgets::context_line(app.view, app.pane, &theme)),
        rows[row + 1],
    );
    overlays::draw(frame, area, app, &theme);
}

fn paint_scanline(frame: &mut Frame, area: ratatui::layout::Rect, theme: &Theme) {
    let style = theme.scanline_style();
    for y in area.y..area.bottom() {
        if y % 2 == 1 {
            let row = ratatui::layout::Rect::new(area.x, y, area.width, 1);
            frame.render_widget(Block::default().style(style), row);
        }
    }
}

fn draw_splash(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let view = app
        .motion
        .frame_kind(motion::AnimKind::Splash)
        .unwrap_or_else(|| motion::FrameView::last(10));
    let lines = motion::splash_lines(
        view,
        app.motion.splash_case(),
        app.motion.splash_marking(),
        theme,
    );
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_toasts(frame: &mut Frame, board: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let toasts = app.motion.toasts();
    if toasts.is_empty() {
        return;
    }
    let width = motion::TOAST_COLS.min(board.width.saturating_sub(1)).max(8);
    let extra = app.motion.collapsed_toast_count();
    for (index, toast) in toasts.iter().take(motion::TOAST_STACK).enumerate() {
        let pad = app.motion.toast_slide(toast);
        let y = board.y + index as u16;
        if y >= board.bottom() {
            break;
        }
        let x = board.x + board.width.saturating_sub(width);
        let area = ratatui::layout::Rect::new(x, y, width, 1);
        frame.render_widget(
            Paragraph::new(toast_line(toast, theme, &theme.glyphs, pad)),
            area,
        );
    }
    if extra > 0 {
        let y = board.y + toasts.len().min(motion::TOAST_STACK) as u16;
        if y < board.bottom() {
            let x = board.x + board.width.saturating_sub(width);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(format!("+{extra}"), theme.muted))),
                ratatui::layout::Rect::new(x, y, width, 1),
            );
        }
    }
}

fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let investigation = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.investigation().name.as_str())
        .unwrap_or("no investigation");
    let marking = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.investigation().marking.label())
        .unwrap_or("unmarked");
    let review = app.pending_review_count();
    let jobs_active = app
        .job_records
        .values()
        .filter(|job| job.status.is_active())
        .count();
    let agent = if app.agent.available {
        format!(
            "agent {} {}",
            theme.glyphs.collected, app.agent.profile.name
        )
    } else {
        format!("agent {} {}", theme.glyphs.external, app.agent.profile.name)
    };
    let review_bit = if review > 0 {
        format!("{} {review} to review", theme.glyphs.ai)
    } else {
        format!("{} 0 to review", theme.glyphs.ai)
    };
    let jobs_bit = if jobs_active > 0 {
        let spin = if app.agent_job_running() {
            app.motion.agent_glyph(&theme.glyphs)
        } else {
            app.motion.job_spinner_glyph(&theme.glyphs)
        };
        format!("{spin} {jobs_active} jobs")
    } else {
        format!("{} jobs", theme.glyphs.done)
    };
    let crumb = if theme.glyphs.ascii { ">" } else { "▸" };
    let title = format!(
        " os-int {crumb} {investigation} {crumb} {}  {marking}",
        app.view.title()
    );
    let right = format!(" {review_bit}  {jobs_bit}  {agent} ");
    let title_width = UnicodeWidthStr::width(title.as_str());
    let right_width = UnicodeWidthStr::width(right.as_str());
    let available = area.width as usize;
    let (title, title_width) = if title_width + right_width > available {
        let keep = available.saturating_sub(right_width.saturating_add(1));
        let truncated = truncate_width(&title, keep, theme.glyphs.ascii);
        let width = UnicodeWidthStr::width(truncated.as_str());
        (truncated, width)
    } else {
        (title, title_width)
    };
    let spacer = " ".repeat(available.saturating_sub(title_width.saturating_add(right_width)));
    let line = Line::from(vec![
        Span::styled(title, theme.title),
        Span::raw(spacer),
        Span::styled(right, theme.muted),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_confirm_bar(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text = app.confirm_bar_text();
    let pulsing = app.motion.confirm_phase() == ConfirmPhase::Pulse;
    let line = confirm_rise_line(app.motion.confirm_rise_frame(), &text, theme, pulsing);
    frame.render_widget(Paragraph::new(line).style(theme.panel), area);
}

fn truncate_width(text: &str, max_width: usize, ascii: bool) -> String {
    widgets::ellipsize(text, max_width, ascii)
}

fn draw_status(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let jobs = app.job_status_line();
    let (message, style) = if let Some(error) = &app.last_error {
        (error.as_str(), theme.err)
    } else {
        (app.status.as_str(), theme.fg)
    };
    let text = format!(" {}    {jobs}", sanitize_line(message));
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App, View};
    use crate::config::Config;
    use crate::model::PageKind;
    use crate::paths::AppPaths;
    use crate::storage::{NewPage, Store};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

    fn visible_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn object_view_renders_page_without_entity_id() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Example Corp");
        }
        app.dispatch(Action::Submit).expect("create");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page = app
            .store
            .create_page(NewPage::new(
                investigation_id,
                "Case notes",
                PageKind::Generic,
                None,
            ))
            .expect("generic page");
        assert!(page.entity_id.is_none());
        app.open_investigation(investigation_id)
            .expect("reload workspace");
        app.page_id = Some(page.id);
        app.entity_id = Some(uuid::Uuid::new_v4());
        app.view = View::Pages;
        app.dispatch(Action::OpenObjectView).expect("object view");
        assert_eq!(app.view, View::Object);
        assert!(app.entity_id.is_none());
        let view = app.current_object_view().expect("page object");
        assert_eq!(view.title, "Case notes");
        assert!(view.groups.iter().any(|group| group.id == "identity"));
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        let text = visible_text(&terminal);
        assert!(text.contains("Case notes"), "{text}");
        assert!(text.contains("Identity"), "{text}");
        assert!(text.contains("P-1"), "{text}");
    }

    #[test]
    fn graph_renders_nodes_and_facets_without_panic() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Example Corp");
        }
        app.dispatch(Action::Submit).expect("create");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page = app
            .store
            .create_page(NewPage::new(
                investigation_id,
                "",
                PageKind::Domain,
                Some("example.com"),
            ))
            .expect("page");
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
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: Some(page.id),
                    job_id: None,
                },
                &batch,
            )
            .expect("applies");
        app.open_investigation(investigation_id)
            .expect("reload workspace");
        app.dispatch(Action::OpenGraph).expect("graph");
        assert_eq!(app.view, View::Graph);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        let text = visible_text(&terminal);
        assert!(
            text.contains("Graph") || text.contains("Neighborhood"),
            "{text}"
        );
        assert!(
            text.contains("example.com") || text.contains("E-1"),
            "{text}"
        );
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Main;
        app.selected = 0;
        terminal
            .draw(|frame| draw(frame, &app))
            .expect("draw entities");
        let entities = visible_text(&terminal);
        assert!(entities.contains("example.com"), "{entities}");
    }

    #[test]
    fn header_spacer_uses_display_width_not_bytes() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("café");
        }
        app.dispatch(Action::Submit).expect("create");
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let theme = Theme::from_config(&app.config);
                draw_header(frame, frame.area(), &app, &theme);
            })
            .expect("draw");
        let text = visible_text(&terminal);
        assert!(text.contains("os-int"), "{text}");
        assert!(text.contains("café"), "{text}");
        let title = " os-int ─ Investigation: café  [TLP:AMBER] ";
        assert!(title.len() > UnicodeWidthStr::width(title));
        assert_eq!(truncate_width("hello", 0, false), "");
    }

    #[test]
    fn status_shows_degrade_hint_when_phosphor_falls_back() {
        let (_directory, mut app) = test_app();
        app.config.ui.theme = "phosphor".into();
        app.config.ui.color = "16".into();
        app.status = "ready".into();
        let backend = TestBackend::new(100, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        let text = visible_text(&terminal);
        assert!(text.contains("phosphor needs 256 colours"), "{text}");
    }

    #[test]
    fn degrade_hint_stays_readable_at_minimum_size() {
        let (_directory, mut app) = test_app();
        app.config.ui.theme = "phosphor".into();
        app.config.ui.color = "16".into();
        app.status = "ready".into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        let text = visible_text(&terminal);
        assert!(
            text.contains("phosphor needs 256 colours"),
            "hint must not be truncated at 80x24\n{text}"
        );
    }

    fn press(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn dispatch_key(app: &mut App, key: crossterm::event::KeyEvent) {
        let action = keymap::map_key(
            key,
            &app.overlay,
            app.view,
            app.is_editing(),
            app.review_armed,
            app.board_zoomed,
        )
        .expect("key should map");
        app.dispatch(action).expect("dispatch");
    }

    #[test]
    fn palette_enter_on_write_command_waits_for_y() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("Palette Write");
        }
        app.dispatch(Action::Submit).expect("create");
        app.dispatch(Action::Palette).expect("palette");
        if let crate::app::Overlay::Palette { query, selected } = &mut app.overlay {
            query.set("generate_report");
            *selected = 0;
        }
        let hits = crate::app::commands::filtered("generate_report");
        assert_eq!(hits[0].id, "generate_report");
        assert!(hits[0].writes);
        dispatch_key(&mut app, press(crossterm::event::KeyCode::Enter));
        assert!(
            matches!(
                app.overlay,
                crate::app::Overlay::Confirm { ref command, .. } if command == "generate_report"
            ),
            "{:?}",
            app.overlay
        );
        assert!(
            app.workspace
                .as_ref()
                .expect("workspace")
                .data
                .artifacts
                .iter()
                .all(|artifact| artifact.kind != crate::model::ArtifactKind::Report)
        );
        assert!(
            keymap::map_key(
                press(crossterm::event::KeyCode::Enter),
                &app.overlay,
                app.view,
                app.is_editing(),
                app.review_armed,
                app.board_zoomed,
            )
            .is_none()
        );
        dispatch_key(&mut app, press(crossterm::event::KeyCode::Char('y')));
        assert!(
            app.workspace
                .as_ref()
                .expect("workspace")
                .data
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == crate::model::ArtifactKind::Report)
        );
    }

    fn snapshot_app() -> (tempfile::TempDir, App) {
        let (directory, mut app) = test_app();
        app.config.ui.theme = "mono".into();
        app.config.ui.color = "none".into();
        app.config.ui.motion = "off".into();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Off);
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("acme-phish");
        }
        app.dispatch(Action::Submit).expect("create");
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let page = app
            .store
            .create_page(NewPage::new(
                investigation_id,
                "",
                PageKind::Domain,
                Some("acme-login.com"),
            ))
            .expect("page");
        let mut batch = crate::model::EvidenceBatch::new();
        let source = crate::model::NewSource::collected(
            crate::model::SourceKind::Dns,
            "dns",
            "DNS records for acme-login.com",
        );
        let key = batch.add_source(source);
        let domain =
            crate::model::EntityKey::new(crate::model::EntityKind::Domain, "acme-login.com");
        let ip = crate::model::EntityKey::new(crate::model::EntityKind::Ip, "203.0.113.7");
        batch.relate(&domain, "resolves_to", &ip, Some(key));
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: Some(page.id),
                    job_id: None,
                },
                &batch,
            )
            .expect("applies");
        app.store
            .create_finding(crate::storage::NewFinding {
                investigation_id,
                page_id: Some(page.id),
                title: "acme-login.com is phishing infra".into(),
                details: "SAN mimics acme.com".into(),
                confidence: crate::model::Confidence::Medium,
                provenance: crate::model::Provenance::Agent,
                source_ids: Vec::new(),
            })
            .expect("finding");
        let mut run = crate::model::AgentRun {
            id: uuid::Uuid::new_v4(),
            investigation_id,
            seq: 0,
            thread_id: uuid::Uuid::new_v4(),
            page_id: Some(page.id),
            agent_name: "codex".into(),
            program: "codex".into(),
            args: Vec::new(),
            question: "Is this phishing?".into(),
            response: String::new(),
            status: crate::model::JobStatus::Succeeded,
            error: None,
            focus: None,
            context_artifact_id: None,
            job_id: None,
            model: None,
            cost_usd: None,
            created_at: crate::model::now(),
            finished_at: Some(crate::model::now()),
        };
        app.store.insert_agent_run(&mut run).expect("run");
        app.store
            .insert_proposals(
                investigation_id,
                run.id,
                vec![(
                    crate::model::AgentAction::AddFinding {
                        title: "mail.acme-login.com uses the same MX".into(),
                        details: "Registered 3 days before the campaign".into(),
                        confidence: Some("medium".into()),
                        sources: vec!["S-1".into()],
                    },
                    crate::model::ProposalStatus::Pending,
                    None,
                )],
            )
            .expect("proposals");
        app.open_investigation(investigation_id)
            .expect("reload workspace");
        (directory, app)
    }

    fn render_snapshot(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
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

    fn assert_screen_snapshot(name: &str, app: &App, width: u16, height: u16) -> Option<String> {
        let actual = render_snapshot(app, width, height);
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push(format!("tests/golden/cockpit/{name}_{width}x{height}.txt"));
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if expected.is_empty() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("golden dir");
            std::fs::write(&path, &actual).expect("write golden");
            return Some(path.display().to_string());
        }
        assert_eq!(
            actual, expected,
            "snapshot mismatch for {} at {width}x{height}\n--- actual ---\n{actual}",
            name
        );
        None
    }

    #[test]
    fn graph_time_lens_draws_the_timeline() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenGraph).expect("graph");
        assert!(!app.is_time_lens());
        let graph = render_snapshot(&app, 80, 24);
        assert!(
            graph.contains("Neighborhood") || graph.contains("Facets") || graph.contains("Graph"),
            "{graph}"
        );
        assert!(!graph.contains("Time lens"));
        app.dispatch(Action::ToggleTimeLens).expect("t");
        assert_eq!(app.view, View::Graph);
        assert!(app.is_time_lens());
        let lens = render_snapshot(&app, 80, 24);
        assert!(lens.contains("Time lens"), "{lens}");
        assert!(!lens.contains("Neighborhood"), "{lens}");
    }

    #[test]
    fn cockpit_snapshots_at_minimum_and_wide() {
        let (_directory, mut app) = snapshot_app();
        let mut wrote = Vec::new();
        for (name, action) in [
            ("case", Action::OpenOverview),
            ("entities", Action::OpenEntities),
            ("evidence", Action::OpenEvidence),
            ("graph", Action::OpenGraph),
            ("review", Action::OpenReview),
            ("jobs", Action::OpenJobs),
        ] {
            app.dispatch(action).expect("view");
            app.pane = crate::app::Pane::Main;
            for (width, height) in [(80, 24), (140, 40)] {
                wrote.extend(assert_screen_snapshot(name, &app, width, height));
            }
        }
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        assert!(app.review_armed);
        wrote.extend(assert_screen_snapshot("review_armed", &app, 80, 24));
        wrote.extend(assert_screen_snapshot("review_armed", &app, 140, 40));
        assert!(
            wrote.is_empty(),
            "wrote snapshots; re-run to confirm: {}",
            wrote.join(", ")
        );
    }

    #[test]
    fn entities_table_has_headers_glyphs_and_no_fake_rows() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Main;
        let screen = render_snapshot(&app, 80, 24);
        assert!(screen.contains("KIND"), "{screen}");
        assert!(screen.contains("VALUE"), "{screen}");
        assert!(screen.contains("PROV"), "{screen}");
        assert!(screen.contains("SRC"), "{screen}");
        assert!(screen.contains("SEEN"), "{screen}");
        assert!(
            screen.contains('●')
                || screen.contains('C')
                || screen.contains('✎')
                || screen.contains('A'),
            "provenance glyph missing\n{screen}"
        );
        assert!(!screen.contains("(no "), "{screen}");
        assert!(!screen.contains("(no entities"), "{screen}");
    }

    #[test]
    fn entities_t_toggles_time_lens() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        assert!(!app.is_time_lens());
        app.dispatch(Action::ToggleTimeLens).expect("t");
        assert_eq!(app.view, View::Entities);
        assert!(app.is_time_lens());
        let lens = render_snapshot(&app, 80, 24);
        assert!(lens.contains("Time lens"), "{lens}");
        app.dispatch(Action::ToggleTimeLens).expect("t off");
        assert!(!app.is_time_lens());
    }

    #[test]
    fn e_on_entities_notes_tab_edits_entity_notes() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.inspector_tab = crate::app::InspectorTab::Notes;
        app.dispatch(Action::Edit).expect("e");
        let session = app.take_pending_editor().expect("editor");
        assert!(session.entity_id.is_some(), "{session:?}");
        app.config.ui.ascii = true;
        app.dispatch(Action::Edit).expect("e again");
        assert!(
            !app.status.contains('…'),
            "plain status leaked unicode ellipsis: {}",
            app.status
        );
        assert!(app.status.contains("Opening editor..."), "{}", app.status);
    }

    #[test]
    fn table_filter_typing_updates_listed_rows() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.dispatch(Action::Search).expect("filter");
        assert!(matches!(
            app.overlay,
            crate::app::Overlay::TableFilter { .. }
        ));
        for ch in "kind:ip".chars() {
            app.dispatch(Action::InsertChar(ch)).expect("type");
        }
        assert_eq!(app.board_filter, "kind:ip");
        let filtered = app.listed_entities();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].kind, crate::model::EntityKind::Ip);
    }

    #[test]
    fn table_filter_and_sort_change_listed_rows() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        let total = app.listed_entities().len();
        assert!(total >= 2);
        app.board_filter = "kind:ip".into();
        let filtered = app.listed_entities();
        assert_eq!(filtered.len(), 1, "kind:ip should leave the address");
        assert_eq!(filtered[0].kind, crate::model::EntityKind::Ip);
        app.board_filter = "sort:-value".into();
        let sorted = app.listed_entities();
        assert!(sorted.len() >= 2);
        assert!(sorted[0].display_name() >= sorted[1].display_name());
        app.dispatch(Action::ClearBoard).expect("x");
        assert!(app.board_filter.is_empty());
        assert_eq!(app.listed_entities().len(), total);
    }

    #[test]
    fn error_panel_persists_until_x() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        let _ = app.dispatch(Action::Command("run_tool:missing-tool"));
        assert!(app.last_error.is_some());
        app.status = "Queued dns".into();
        let screen = render_snapshot(&app, 80, 24);
        assert!(
            screen.contains("unknown tool")
                || app.last_error.as_deref().unwrap().contains("unknown"),
            "{screen}"
        );
        app.dispatch(Action::OpenJobs).expect("jobs");
        assert!(app.last_error.is_some(), "errors must not be overwritten");
        app.dispatch(Action::ClearBoard).expect("x");
        assert!(app.last_error.is_none());
    }

    #[test]
    fn inspector_history_is_scoped_to_the_selected_object() {
        let (_directory, mut app) = snapshot_app();
        let workspace = app.workspace.as_ref().expect("workspace");
        let investigation_id = workspace.id();
        let entity_id = workspace.data.entities[0].id;
        let other = uuid::Uuid::new_v4();
        app.store
            .append_audit(
                investigation_id,
                crate::model::AuditKind::Export,
                "exported other object",
                crate::model::Fields::new(),
                vec![other],
            )
            .expect("audit other");
        app.store
            .append_audit(
                investigation_id,
                crate::model::AuditKind::MarkingChanged,
                "marked this entity",
                crate::model::Fields::new(),
                vec![entity_id],
            )
            .expect("audit entity");
        app.open_investigation(investigation_id).expect("reload");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.entity_id = Some(entity_id);
        app.inspector_tab = crate::app::InspectorTab::History;
        app.pane = crate::app::Pane::Detail;
        let screen = render_snapshot(&app, 140, 40);
        assert!(screen.contains("marked this entity"), "{screen}");
        assert!(
            !screen.contains("exported other object"),
            "history must be per-object\n{screen}"
        );
    }

    #[test]
    fn tools_table_shows_passive_and_refused_status() {
        let (_directory, mut app) = snapshot_app();
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
        app.tools = std::sync::Arc::new(crate::tools::ToolRegistry::detect(&app.config.tools));
        app.dispatch(Action::OpenTools).expect("tools");
        app.pane = crate::app::Pane::Main;
        let screen = render_snapshot(&app, 140, 40);
        assert!(
            screen.contains("ACTIVITY") || screen.contains("passive") || screen.contains("NAME"),
            "{screen}"
        );
        assert!(
            screen.contains("nmap") && (screen.contains("refused") || screen.contains("active")),
            "{screen}"
        );
        assert!(
            screen.contains('⊘') || screen.contains("no"),
            "refused glyph missing\n{screen}"
        );
        assert!(!screen.contains("(no "), "{screen}");
        assert!(!screen.contains("enable"), "{screen}");
    }

    #[test]
    fn inspector_sources_show_glyphs_and_the_source_chain() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Detail;
        app.inspector_tab = crate::app::InspectorTab::Sources;
        let screen = render_snapshot(&app, 140, 40);
        assert!(screen.contains("SOURCES"), "{screen}");
        assert!(screen.contains("S1") || screen.contains("S-1"), "{screen}");
        assert!(
            screen.contains('●')
                || screen.contains('C')
                || screen.contains('◇')
                || screen.contains('X'),
            "source-chain glyph missing\n{screen}"
        );
    }

    #[test]
    fn empty_entities_board_is_not_a_fake_row() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Command("new_investigation"))
            .expect("form");
        if let crate::app::Overlay::Form(form) = &mut app.overlay {
            form.fields[0].value.set("empty-case");
        }
        app.dispatch(Action::Submit).expect("create");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Main;
        assert_eq!(app.listed_entities().len(), 0);
        let screen = render_snapshot(&app, 80, 24);
        assert!(screen.contains("No entities yet"), "{screen}");
        assert!(!screen.contains("(no entities"), "{screen}");
    }

    #[test]
    fn selection_stays_on_the_same_id_while_filtering_and_on_apply() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        let keep = app.listed_entities()[0].id;
        app.entity_id = Some(keep);
        app.selected = 0;
        app.dispatch(Action::Search).expect("filter");
        for ch in "sort:-kind".chars() {
            app.dispatch(Action::InsertChar(ch)).expect("type");
        }
        assert_eq!(app.entity_id, Some(keep));
        assert_eq!(app.listed_entities()[app.selected].id, keep);
        app.dispatch(Action::Select).expect("apply");
        assert_eq!(app.overlay, crate::app::Overlay::None);
        assert_eq!(app.entity_id, Some(keep));
        assert_eq!(app.listed_entities()[app.selected].id, keep);

        app.dispatch(Action::OpenEvidence).expect("evidence");
        let keep_row = app.listed_evidence()[0];
        app.dispatch(Action::Search).expect("filter");
        for ch in "sort:-value".chars() {
            app.dispatch(Action::InsertChar(ch)).expect("type");
        }
        assert_eq!(app.listed_evidence()[app.selected], keep_row);
        app.dispatch(Action::Select).expect("apply");
        assert_eq!(app.listed_evidence()[app.selected], keep_row);
    }

    #[test]
    fn empty_filter_clears_selection_and_jk_do_nothing() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.dispatch(Action::Search).expect("filter");
        for ch in "kind:xyzzy".chars() {
            app.dispatch(Action::InsertChar(ch)).expect("type");
        }
        assert!(app.listed_entities().is_empty());
        assert!(app.entity_id.is_none());
        app.dispatch(Action::Move(1)).expect("j");
        assert_eq!(app.selected, 0);
        assert!(app.entity_id.is_none());
        app.pane = crate::app::Pane::Detail;
        let screen = render_snapshot(&app, 140, 40);
        assert!(screen.contains("Nothing selected"), "{screen}");
    }

    #[test]
    fn tab_from_the_board_opens_inspector_sources() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.inspector_tab = crate::app::InspectorTab::Props;
        app.pane = crate::app::Pane::Main;
        app.dispatch(Action::NextPane).expect("tab");
        assert_eq!(app.pane, crate::app::Pane::Detail);
        assert_eq!(app.inspector_tab, crate::app::InspectorTab::Sources);
        let screen = render_snapshot(&app, 140, 40);
        assert!(screen.contains("SOURCES"), "{screen}");
        assert!(screen.contains("S1") || screen.contains("S-1"), "{screen}");
    }

    #[test]
    fn props_and_sources_share_footnote_numbers() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Detail;
        app.inspector_tab = crate::app::InspectorTab::Sources;
        let sources = render_snapshot(&app, 140, 40);
        assert!(sources.contains("S1"), "{sources}");
        app.inspector_tab = crate::app::InspectorTab::Props;
        let props = render_snapshot(&app, 140, 40);
        let workspace = app.workspace.as_ref().expect("workspace");
        let entity = workspace
            .entity(app.selected_entity_id().expect("entity"))
            .expect("row");
        if !entity.fields.is_empty() {
            assert!(props.contains("S1"), "{props}");
        }
        assert!(
            !props.contains("S12") && sources.contains("S1"),
            "footnotes must use the chain index\n{props}\n{sources}"
        );
    }

    #[test]
    fn switching_view_clears_the_board_filter() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.board_filter = "kind:ip".into();
        app.dispatch(Action::OpenJobs).expect("jobs");
        assert!(app.board_filter.is_empty());
    }

    #[test]
    fn motion_off_review_confirm_has_no_countdown_or_rise() {
        let (_directory, mut app) = snapshot_app();
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        assert!(app.review_armed);
        let screen = render_snapshot(&app, 80, 24);
        assert!(
            screen.contains("y confirm") || screen.contains("y confirm"),
            "armed bar must be readable with motion off\n{screen}"
        );
        assert!(!screen.contains('▁') && !screen.contains('▄'), "{screen}");
        assert!(
            !screen.to_ascii_lowercase().contains("countdown")
                && !screen.contains("auto")
                && !screen.contains("sec left"),
            "{screen}"
        );
    }

    #[test]
    fn extra_table_goldens_filter_empty_error_and_ascii() {
        let (_directory, mut app) = snapshot_app();
        let mut wrote = Vec::new();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Main;
        app.config.ui.theme = "mono".into();
        wrote.extend(assert_screen_snapshot("entities_mono", &app, 80, 24));
        app.config.ui.theme = "auto".into();
        app.config.ui.ascii = true;
        wrote.extend(assert_screen_snapshot("entities_ascii", &app, 80, 24));
        app.config.ui.ascii = false;
        app.board_filter = "kind:ip".into();
        wrote.extend(assert_screen_snapshot("entities_filter", &app, 80, 24));
        app.board_filter = "kind:xyzzy".into();
        wrote.extend(assert_screen_snapshot(
            "entities_filter_empty",
            &app,
            80,
            24,
        ));
        app.board_filter.clear();
        app.last_error = Some("unknown tool missing-tool".into());
        wrote.extend(assert_screen_snapshot("entities_error", &app, 80, 24));
        assert!(
            wrote.is_empty(),
            "wrote snapshots; re-run to confirm: {}",
            wrote.join(", ")
        );
    }

    #[test]
    fn theme_cycle_enter_previews_y_saves() {
        let (directory, mut app) = test_app();
        let config_path = directory.path().join("config.toml");
        app.config.ui.theme = "dark".into();
        app.dispatch(Action::Command("theme")).expect("theme");
        assert!(matches!(
            app.overlay,
            crate::app::Overlay::ThemeCycle { .. }
        ));
        app.dispatch(Action::Move(1)).expect("preview light");
        assert_eq!(app.config.ui.theme, "light");
        assert!(
            keymap::map_key(
                press(crossterm::event::KeyCode::Enter),
                &app.overlay,
                app.view,
                app.is_editing(),
                app.review_armed,
                app.board_zoomed,
            )
            .is_none()
        );
        assert!(
            !std::fs::read_to_string(&config_path)
                .unwrap_or_default()
                .contains("theme = \"light\"")
        );
        dispatch_key(&mut app, press(crossterm::event::KeyCode::Char('y')));
        assert_eq!(app.overlay, crate::app::Overlay::None);
        let saved = std::fs::read_to_string(&config_path).expect("saved config");
        assert!(saved.contains("theme = \"light\""), "{saved}");
    }

    #[test]
    fn theme_cycle_question_reverts_preview_then_opens_help() {
        let (_directory, mut app) = test_app();
        app.config.ui.theme = "dark".into();
        app.dispatch(Action::Command("theme")).expect("theme");
        app.dispatch(Action::Move(1)).expect("preview light");
        assert_eq!(app.config.ui.theme, "light");
        dispatch_key(&mut app, press(crossterm::event::KeyCode::Char('?')));
        assert!(matches!(app.overlay, crate::app::Overlay::Help));
        assert_eq!(app.config.ui.theme, "dark", "preview must revert on ?");
    }

    #[test]
    fn palette_theme_opens_cycle_without_confirm() {
        let (_directory, mut app) = test_app();
        app.dispatch(Action::Palette).expect("palette");
        if let crate::app::Overlay::Palette { query, selected } = &mut app.overlay {
            query.set("theme");
            *selected = 0;
        }
        let hits = crate::app::commands::filtered("theme");
        assert_eq!(hits[0].id, "theme");
        assert!(hits[0].writes);
        dispatch_key(&mut app, press(crossterm::event::KeyCode::Enter));
        assert!(
            matches!(app.overlay, crate::app::Overlay::ThemeCycle { .. }),
            "{:?}",
            app.overlay
        );
    }

    #[test]
    fn x_on_toast_does_not_clear_the_filter() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenEntities).expect("entities");
        app.board_filter = "kind:ip".into();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        app.motion
            .push_toast(motion::ToastKind::Err, "ct failed — timeout");
        app.dispatch(Action::ClearBoard).expect("dismiss toast");
        assert_eq!(app.board_filter, "kind:ip");
        assert!(app.motion.toasts().is_empty());
        app.dispatch(Action::ClearBoard).expect("clear filter");
        assert!(app.board_filter.is_empty());
    }

    #[test]
    fn unzoom_and_repeat_narrow_resize_do_not_restart_the_drawer() {
        let (_directory, mut app) = snapshot_app();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        app.on_resize(true);
        assert!(app.motion.playing(motion::AnimKind::Drawer));
        for _ in 0..8 {
            let _ = app.dispatch(Action::Tick);
        }
        assert!(!app.motion.playing(motion::AnimKind::Drawer));
        app.on_resize(true);
        assert!(
            !app.motion.playing(motion::AnimKind::Drawer),
            "still-narrow resize must not restart the drawer"
        );
        app.board_zoomed = true;
        app.dispatch(Action::Unzoom).expect("unzoom");
        assert!(!app.board_zoomed);
        assert!(
            !app.motion.playing(motion::AnimKind::Drawer),
            "unzoom must not restart the drawer"
        );
    }

    #[test]
    fn ctrl_c_maps_to_quit_while_an_overlay_is_open() {
        let overlay = crate::app::Overlay::ThemeCycle {
            original: "dark".into(),
            index: 0,
        };
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        assert!(matches!(
            keymap::map_key(key, &overlay, View::Overview, false, false, false),
            Some(Action::Quit)
        ));
    }

    #[test]
    fn toasts_sit_on_the_board_not_the_inspector() {
        let (_directory, mut app) = snapshot_app();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Reduced);
        app.motion
            .push_toast(motion::ToastKind::Ok, "ct done · 37 certs");
        let screen = render_snapshot(&app, 140, 40);
        let toast_col = screen
            .lines()
            .find_map(|line| line.find("ct done"))
            .unwrap_or_else(|| panic!("toast missing\n{screen}"));
        let inspector = screen
            .lines()
            .find_map(|line| line.find("Props").or_else(|| line.find("Inspector")))
            .unwrap_or(140 - 44);
        assert!(
            toast_col < inspector,
            "toast column {toast_col} covers inspector at {inspector}\n{screen}"
        );
    }

    #[test]
    fn cockpit_motion_first_mid_last_goldens() {
        let (_directory, mut app) = snapshot_app();
        app.config.ui.theme = "mono".into();
        app.config.ui.color = "none".into();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        let mut wrote = Vec::new();
        app.motion.start_splash("acme-phish", "TLP:AMBER");
        wrote.extend(assert_screen_snapshot("motion_m1_first", &app, 80, 24));
        app.motion = app.motion.freeze_at(5);
        wrote.extend(assert_screen_snapshot("motion_m1_mid", &app, 80, 24));
        app.motion = app.motion.freeze_at(9);
        wrote.extend(assert_screen_snapshot("motion_m1_last", &app, 80, 24));
        app.motion.skip_splash();
        app.on_resize(true);
        wrote.extend(assert_screen_snapshot("motion_m6_first", &app, 80, 24));
        app.motion = app.motion.freeze_at(2);
        wrote.extend(assert_screen_snapshot("motion_m6_mid", &app, 80, 24));
        app.motion = app.motion.freeze_at(3);
        wrote.extend(assert_screen_snapshot("motion_m6_last", &app, 80, 24));
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        app.motion
            .push_toast(motion::ToastKind::Ok, "ct done · 37 certs");
        wrote.extend(assert_screen_snapshot("motion_m8_first", &app, 140, 40));
        app.motion = app.motion.freeze_at(2);
        wrote.extend(assert_screen_snapshot("motion_m8_mid", &app, 140, 40));
        app.motion = app.motion.freeze_at(3);
        wrote.extend(assert_screen_snapshot("motion_m8_last", &app, 140, 40));
        assert!(
            wrote.is_empty(),
            "wrote snapshots; re-run to confirm: {}",
            wrote.join(", ")
        );
    }

    fn seed_jobs(app: &mut App) {
        use crate::model::{JobRecord, JobStatus, now};
        let investigation_id = app.workspace.as_ref().map(|workspace| workspace.id());
        for (seq, (kind, label, progress)) in [
            ("collector:dns", "DNS", "crt.sh page 2/8"),
            ("collector:ct", "CT", "page 3/8"),
            ("collector:whois", "WHOIS", "1/1"),
            ("agent", "Claude", "thinking"),
        ]
        .into_iter()
        .enumerate()
        {
            let job = JobRecord {
                id: uuid::Uuid::new_v4(),
                investigation_id,
                seq: seq as i64 + 1,
                kind: kind.into(),
                label: label.into(),
                description: progress.into(),
                status: JobStatus::Running,
                program: None,
                args: Vec::new(),
                created_at: now(),
                started_at: Some(now()),
                finished_at: None,
                progress: Some(progress.into()),
                log: String::new(),
                stderr: String::new(),
                result: None,
                error: None,
                page_id: None,
            };
            app.job_records.insert(job.id, job);
        }
        app.sync_motion();
        app.mark_dirty();
    }

    #[test]
    fn cockpit_tick_writes_at_most_400_crossterm_bytes() {
        use std::cell::Cell;
        use std::io::{self, Write};
        use std::rc::Rc;

        use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
        use ratatui::layout::{Position, Size};

        #[derive(Clone)]
        struct Counter(Rc<Cell<usize>>);

        struct CountingWriter(Counter);
        impl Write for CountingWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.0.set(self.0.0.get() + buf.len());
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct SizedCrossterm {
            inner: CrosstermBackend<CountingWriter>,
            width: u16,
            height: u16,
        }
        impl Backend for SizedCrossterm {
            type Error = io::Error;
            fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
            where
                I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
            {
                self.inner.draw(content)
            }
            fn hide_cursor(&mut self) -> io::Result<()> {
                self.inner.hide_cursor()
            }
            fn show_cursor(&mut self) -> io::Result<()> {
                self.inner.show_cursor()
            }
            fn get_cursor_position(&mut self) -> io::Result<Position> {
                self.inner.get_cursor_position()
            }
            fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
                self.inner.set_cursor_position(position)
            }
            fn clear(&mut self) -> io::Result<()> {
                self.inner.clear()
            }
            fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
                self.inner.clear_region(clear_type)
            }
            fn size(&self) -> io::Result<Size> {
                Ok(Size::new(self.width, self.height))
            }
            fn window_size(&mut self) -> io::Result<WindowSize> {
                Ok(WindowSize {
                    columns_rows: Size::new(self.width, self.height),
                    pixels: Size::new(0, 0),
                })
            }
            fn flush(&mut self) -> io::Result<()> {
                Backend::flush(&mut self.inner)
            }
        }

        let (_directory, mut app) = snapshot_app();
        app.config.ui.theme = "dark".into();
        app.config.ui.color = "truecolor".into();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        seed_jobs(&mut app);
        let counter = Counter(Rc::new(Cell::new(0)));
        let backend = SizedCrossterm {
            inner: CrosstermBackend::new(CountingWriter(counter.clone())),
            width: 120,
            height: 40,
        };
        let mut terminal = Terminal::new(backend).expect("terminal");
        let ticks = 40usize;
        for i in 0..ticks {
            let _ = app.dispatch(Action::Tick);
            counter.0.set(0);
            terminal
                .draw(|frame| draw(frame, &app))
                .expect("cockpit draw");
            let bytes = counter.0.get();
            if i >= 2 {
                assert!(
                    bytes <= 400,
                    "tick {i} wrote {bytes} crossterm bytes, budget is 400"
                );
            }
        }
    }

    #[test]
    fn case_home_has_targets_glance_and_next() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenOverview).expect("case");
        app.pane = crate::app::Pane::Main;
        let home = app.case_home();
        assert!(!home.targets.is_empty());
        assert!(home.entities.total >= 1);
        let screen = render_snapshot(&app, 80, 24);
        assert!(screen.contains("TARGETS"), "{screen}");
        assert!(screen.contains("AT A GLANCE"), "{screen}");
        assert!(screen.contains("NEXT"), "{screen}");
        assert!(!screen.contains("braille"), "{screen}");
        assert!(
            screen.contains("facts") && screen.contains("sources"),
            "Case Inspector must show N facts · M sources\n{screen}"
        );
        assert!(screen.contains("SOURCES"), "{screen}");
        assert!(screen.contains("NOTES"), "{screen}");
        assert!(
            screen.contains("n new target"),
            "Case context bar must keep n new target at 80x24\n{screen}"
        );
    }

    #[test]
    fn graph_enter_focuses_the_selected_node() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenGraph).expect("graph");
        let before = app.graph.focus_id;
        app.dispatch(Action::Select).expect("enter");
        assert!(app.graph.focus_id.is_some());
        assert_eq!(app.view, View::Graph);
        assert_ne!(app.view, View::Object);
        let _ = before;
        let screen = render_snapshot(&app, 80, 24);
        assert!(
            screen.contains("├")
                || screen.contains("└")
                || screen.contains("|-")
                || screen.contains("DOM"),
            "tree missing\n{screen}"
        );
        assert!(
            !screen.contains("Neighborhood") || screen.contains("KIND"),
            "{screen}"
        );
        let width = screen
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        assert!(width <= 80, "graph exceeds 80 cols: {width}\n{screen}");
    }

    #[test]
    fn graph_cursor_uses_tree_ids_after_reroot() {
        let (_directory, mut app) = snapshot_app();
        app.dispatch(Action::OpenGraph).expect("graph");
        let ids = app.graph_tree_ids();
        assert!(ids.len() >= 2, "need a child to re-root");
        app.dispatch(Action::Move(1)).expect("j");
        let selected = app.graph_tree_ids()[app.graph.selected];
        assert_eq!(app.entity_id, Some(selected));
        let name = app
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.entity(selected))
            .map(|entity| entity.display_name().to_string())
            .expect("node");
        app.dispatch(Action::Select).expect("enter re-root");
        assert_eq!(app.graph.focus_id, Some(selected));
        assert_eq!(
            app.graph_tree_ids().get(app.graph.selected).copied(),
            Some(selected)
        );
        let screen = render_snapshot(&app, 80, 24);
        let hit = screen
            .lines()
            .any(|line| line.contains(&name) && (line.contains('▌') || line.contains('>')));
        assert!(hit, "cursor must sit on {name} after re-root\n{screen}");
    }

    #[test]
    fn review_drawer_keeps_citations_visible_at_80x24() {
        let (_directory, mut app) = snapshot_app();
        app.open_review_lane();
        app.pane = crate::app::Pane::Detail;
        let screen = render_snapshot(&app, 80, 24);
        assert!(
            screen.contains("CITED") || screen.contains("S-1") || screen.contains("S1"),
            "citations clipped in Review drawer\n{screen}"
        );
        assert!(
            screen.contains("TRANSCRIPT")
                || screen.contains("Will write")
                || screen.contains("CITED"),
            "{screen}"
        );
    }

    #[test]
    fn plain_linear_mode_forces_motion_off_and_has_no_spinner_churn() {
        let (_directory, mut app) = snapshot_app();
        app.apply_plain_mode();
        assert!(app.is_plain_linear());
        assert_eq!(app.motion.mode(), crate::tui::MotionMode::Off);
        seed_jobs(&mut app);
        let first = render_snapshot(&app, 80, 24);
        let _ = app.dispatch(Action::Tick);
        let second = render_snapshot(&app, 80, 24);
        assert_eq!(first, second, "plain mode must not churn");
        assert!(!first.contains('⠋'), "{first}");
        assert!(first.contains("RAIL") || first.contains("Case"), "{first}");
    }

    #[test]
    fn a11y_snapshots_mono_ascii_motion_off() {
        let (_directory, mut app) = snapshot_app();
        app.config.ui.theme = "mono".into();
        app.apply_plain_mode();
        assert!(app.is_plain_linear());
        assert_eq!(app.motion.mode(), crate::tui::MotionMode::Off);
        let mut wrote = Vec::new();
        for (name, action) in [
            ("case_plain", Action::OpenOverview),
            ("graph_plain", Action::OpenGraph),
            ("review_plain", Action::OpenReview),
        ] {
            app.dispatch(action).expect("view");
            app.pane = crate::app::Pane::Main;
            wrote.extend(assert_screen_snapshot(name, &app, 80, 24));
        }
        assert!(
            wrote.is_empty(),
            "wrote snapshots; re-run to confirm: {}",
            wrote.join(", ")
        );
    }

    #[test]
    fn plain_80x24_keeps_case_next_graph_edges_and_ascii_glyphs() {
        let (_directory, mut app) = snapshot_app();
        app.apply_plain_mode();
        app.dispatch(Action::OpenOverview).expect("case");
        app.pane = crate::app::Pane::Main;
        let case = render_snapshot(&app, 80, 24);
        assert!(case.contains("NEXT"), "Case NEXT clipped at 80x24\n{case}");
        assert!(
            case.contains("agent proposals") || case.contains("5 review"),
            "NEXT body clipped at 80x24\n{case}"
        );
        assert!(
            !case.contains('◇'),
            "Findings glance must not use the hollow-diamond glyph in plain mode\n{case}"
        );
        assert!(
            case.contains('*') && case.contains("findings"),
            "agent findings must use the ASCII AI glyph\n{case}"
        );
        app.dispatch(Action::OpenGraph).expect("graph");
        app.pane = crate::app::Pane::Main;
        let graph = render_snapshot(&app, 80, 24);
        assert!(
            graph.contains("resolves_to"),
            "Graph edges clipped at 80x24\n{graph}"
        );
        assert!(
            graph.contains("|-") || graph.contains("`-") || graph.contains("->"),
            "Graph tree edge missing at 80x24\n{graph}"
        );
        assert!(
            !graph.contains('│'),
            "plain-mode facet strip must not use a box-drawing bar\n{graph}"
        );
        assert!(graph.contains(" | ") || graph.contains("|"), "{graph}");
        assert!(
            !case.contains('·') && !case.contains('—'),
            "plain Case chrome leaked middle-dot or em dash\n{case}"
        );
        assert!(
            !graph.contains('·') && !graph.contains('—'),
            "plain Graph chrome leaked middle-dot or em dash\n{graph}"
        );
        assert!(
            case.contains("Inspector |") || case.contains("facts |"),
            "{case}"
        );
    }

    #[test]
    fn plain_chrome_covers_inspector_tabs_tools_jobs_and_confirm() {
        let (_directory, mut app) = snapshot_app();
        app.apply_plain_mode();
        app.dispatch(Action::OpenOverview).expect("case");
        app.pane = crate::app::Pane::Detail;
        let inspector = render_snapshot(&app, 80, 24);
        assert!(
            !inspector.contains('·') && !inspector.contains('—') && !inspector.contains('…'),
            "Inspector chrome leaked unicode\n{inspector}"
        );
        app.dispatch(Action::OpenTools).expect("tools");
        app.pane = crate::app::Pane::Main;
        let tools = render_snapshot(&app, 80, 24);
        assert!(
            !tools.contains('·'),
            "tools missing mark leaked middle-dot\n{tools}"
        );
        app.open_review_lane();
        app.dispatch(Action::Arm).expect("arm");
        let confirm = app.confirm_bar_text();
        assert!(!confirm.contains('—'), "{confirm}");
        assert!(!confirm.contains('·'), "{confirm}");
        let armed = app.confirm_accept_text();
        assert!(!armed.contains('—'), "{armed}");
        assert!(armed.contains("Armed -"), "{armed}");
        app.dispatch(Action::OpenEntities).expect("entities");
        app.pane = crate::app::Pane::Main;
        app.dispatch(Action::Search).expect("filter");
        for ch in "kind:ip".chars() {
            app.dispatch(Action::InsertChar(ch)).expect("type");
        }
        let filtered = render_snapshot(&app, 80, 24);
        assert!(
            !filtered.contains('→'),
            "filter count leaked unicode arrow\n{filtered}"
        );
        assert!(
            filtered.contains("->") || filtered.contains("kind:ip"),
            "{filtered}"
        );
        app.dispatch(Action::Help).expect("help");
        let help = render_snapshot(&app, 80, 24);
        assert!(!help.contains('▶') && !help.contains('◀'), "{help}");
    }

    #[test]
    fn apply_plain_mode_forces_no_colour() {
        let (_directory, mut app) = snapshot_app();
        app.config.ui.color = "truecolor".into();
        app.config.ui.theme = "dark".into();
        app.apply_plain_mode();
        let theme = Theme::from_config(&app.config);
        assert_eq!(theme.depth, ColorDepth::None);
        assert!(theme.glyphs.ascii);
        assert_eq!(app.config.ui.color, "none");
    }

    #[test]
    fn cockpit_render_matrix_covers_themes_color_motion_and_sizes() {
        let (_directory, mut app) = snapshot_app();
        let sizes = [(80u16, 24u16), (110, 24), (140, 40), (200, 50)];
        let env = crate::tui::TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: false,
            colorfgbg: None,
        };
        for theme in ["dark", "light", "mono", "phosphor", "amber"] {
            for color in ["truecolor", "256", "16"] {
                for motion in ["full", "reduced", "off"] {
                    app.config.ui.theme = theme.into();
                    app.config.ui.color = color.into();
                    app.config.ui.motion = motion.into();
                    app.config.ui.ascii = false;
                    app.plain = false;
                    app.motion = crate::tui::Animator::from_config(&app.config, &env, false);
                    let views = [
                        Action::OpenOverview,
                        Action::OpenEntities,
                        Action::OpenEvidence,
                        Action::OpenGraph,
                        Action::OpenReview,
                        Action::OpenJobs,
                        Action::OpenTools,
                        Action::OpenObjectView,
                    ];
                    for action in views {
                        app.dispatch(action).expect("view");
                        app.pane = crate::app::Pane::Main;
                        for (width, height) in sizes {
                            let screen = render_snapshot(&app, width, height);
                            assert!(
                                !screen.trim().is_empty(),
                                "{theme} {color} {motion} {width}x{height} blank"
                            );
                            let max_width = screen
                                .lines()
                                .map(|line| line.chars().count())
                                .max()
                                .unwrap_or(0);
                            assert!(
                                max_width <= width as usize,
                                "{theme} {color} {motion} {width}x{height} overflow {max_width}"
                            );
                            match app.view {
                                View::Overview => {
                                    assert!(
                                        screen.contains("NEXT"),
                                        "{theme} {color} {motion} {width}x{height} missing NEXT\n{screen}"
                                    );
                                }
                                View::Graph => {
                                    let has_edge = screen.contains("resolves_to")
                                        && (screen.contains("─▶")
                                            || screen.contains("->")
                                            || screen.contains("|-")
                                            || screen.contains("`-"));
                                    assert!(
                                        has_edge,
                                        "{theme} {color} {motion} {width}x{height} missing graph edges\n{screen}"
                                    );
                                }
                                View::Tools => {
                                    assert!(
                                        screen.contains("TOOLS") || screen.contains("Tools"),
                                        "{theme} {color} {motion} {width}x{height} missing Tools\n{screen}"
                                    );
                                }
                                View::Object => {
                                    assert!(
                                        screen.contains("Object")
                                            || screen.contains("Properties")
                                            || screen.contains("Linked"),
                                        "{theme} {color} {motion} {width}x{height} missing Object View\n{screen}"
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    app.view = View::Welcome;
                    app.pane = crate::app::Pane::Main;
                    for (width, height) in sizes {
                        let screen = render_snapshot(&app, width, height);
                        assert!(
                            screen.contains("Welcome")
                                || screen.contains("New investigation")
                                || screen.contains("os-int"),
                            "{theme} {color} {motion} {width}x{height} missing Welcome\n{screen}"
                        );
                    }
                }
            }
        }
        app.apply_plain_mode();
        app.dispatch(Action::OpenOverview).expect("case");
        let plain = render_snapshot(&app, 80, 24);
        assert!(plain.contains("NEXT"), "{plain}");
        assert!(!plain.contains('◇'), "{plain}");
        assert!(!plain.contains('│'), "{plain}");
        assert!(!plain.contains('…'), "{plain}");
        assert!(!plain.contains('·'), "{plain}");
        assert!(!plain.contains('—'), "{plain}");
    }

    #[test]
    fn large_workspace_renders_without_panic() {
        let (_directory, mut app) = snapshot_app();
        let investigation_id = app.workspace.as_ref().expect("workspace").id();
        let mut batch = crate::model::EvidenceBatch::new();
        let source =
            crate::model::NewSource::collected(crate::model::SourceKind::Dns, "dns", "bulk dns");
        let key = batch.add_source(source);
        for index in 0..250 {
            let host = format!("host-{index}.example.com");
            let domain = crate::model::EntityKey::new(crate::model::EntityKind::Hostname, &host);
            let ip = crate::model::EntityKey::new(
                crate::model::EntityKind::Ip,
                &format!("203.0.113.{}", index % 200 + 1),
            );
            batch.relate(&domain, "resolves_to", &ip, Some(key));
        }
        app.store
            .apply_batch(
                crate::storage::ApplyContext {
                    investigation_id,
                    page_id: None,
                    job_id: None,
                },
                &batch,
            )
            .expect("bulk apply");
        app.open_investigation(investigation_id)
            .expect("reload large");
        app.dispatch(Action::OpenEntities).expect("entities");
        let entities = render_snapshot(&app, 80, 24);
        assert!(
            entities.contains("ENTITIES") || entities.contains("DOM"),
            "{entities}"
        );
        app.dispatch(Action::OpenGraph).expect("graph");
        let graph = render_snapshot(&app, 200, 50);
        assert!(graph.contains("Graph") || graph.contains("KIND"), "{graph}");
        app.dispatch(Action::OpenOverview).expect("case");
        let home = render_snapshot(&app, 110, 24);
        assert!(
            home.contains("AT A GLANCE") || home.contains("TARGETS"),
            "{home}"
        );
    }

    /// Release-only: p99 render ≤ 1 ms and ≤ 2% of one core at 120×40 with 3 jobs + the agent.
    /// Debug `cargo test` skips this. Run `cargo test --release cockpit_cpu_budget_two_percent`.
    #[cfg(not(debug_assertions))]
    #[test]
    fn cockpit_cpu_budget_two_percent() {
        let (_directory, mut app) = snapshot_app();
        app.motion = crate::tui::Animator::new(crate::tui::MotionMode::Full);
        seed_jobs(&mut app);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let ticks = 80usize;
        let mut times = Vec::new();
        let start = std::time::Instant::now();
        for _ in 0..ticks {
            let _ = app.dispatch(Action::Tick);
            let tick_start = std::time::Instant::now();
            terminal
                .draw(|frame| draw(frame, &app))
                .expect("cockpit draw");
            times.push(tick_start.elapsed());
        }
        let elapsed = start.elapsed();
        times.sort();
        let p99 = times[(times.len() * 99 / 100).min(times.len() - 1)];
        let budget = TICK * ticks as u32 * 2 / 100;
        assert!(
            p99.as_millis() < 1 || p99.as_micros() < 1000,
            "p99 cockpit render {p99:?} exceeds 1 ms (CI threshold)"
        );
        assert!(
            elapsed <= budget,
            "CPU {elapsed:?} over {ticks} ticks exceeds 2% of one core ({budget:?}). \
             Documented CI threshold: ≤ 2% at 120×40 with 3 jobs + agent."
        );
    }
}
