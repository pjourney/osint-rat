//! Terminal event loop and rendering. Drawing is a pure function of `App`.

mod glyphs;
mod keymap;
mod overlays;
mod preview;
mod screens;
mod theme;
mod widgets;

pub use preview::render_preview;
pub use theme::{ColorDepth, ColorMode, TerminalEnv, Theme, ThemeName};

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
    let tick_rate = Duration::from_millis(app.config.ui.tick_rate_ms.max(50));
    let mut ticker = tokio::time::interval(tick_rate);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let shutdown = wait_for_shutdown();
    tokio::pin!(shutdown);
    app.mark_dirty();
    while !app.should_quit {
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
                        if let Some(action) = keymap::map_key(key, &app.overlay, app.view, app.is_editing()) {
                            dispatch(app, action);
                            if let Err(error) = run_external_editor(terminal, app) {
                                app.status = error.to_string();
                                app.mark_dirty();
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => app.mark_dirty(),
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
            _ = ticker.tick(), if app.has_active_jobs() => {
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
    if let Err(error) = app.dispatch(action) {
        let message = error.to_string();
        app.last_error = Some(message.clone());
        app.status = message;
        app.mark_dirty();
    }
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
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    draw_header(frame, rows[0], app, &theme);
    if app.view == View::Welcome {
        screens::welcome(frame, rows[1], app, &theme);
    } else {
        screens::workspace(frame, rows[1], app, &theme);
    }
    draw_status(frame, rows[2], app, &theme);
    frame.render_widget(Paragraph::new(widgets::hint_line(&theme)), rows[3]);
    overlays::draw(frame, area, app, &theme);
}

fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let investigation = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.investigation().name.as_str())
        .unwrap_or("no investigation");
    let agent = if app.agent.available {
        format!("{} ●", app.agent.profile.name)
    } else {
        format!("{} ○", app.agent.profile.name)
    };
    let marking = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.investigation().marking.label())
        .unwrap_or("unmarked");
    let title = format!(" os-int ─ Investigation: {investigation}  [{marking}] ");
    let right = format!(" {agent} ");
    let title_width = UnicodeWidthStr::width(title.as_str());
    let right_width = UnicodeWidthStr::width(right.as_str());
    let available = area.width as usize;
    let (title, title_width) = if title_width + right_width > available {
        let keep = available.saturating_sub(right_width.saturating_add(1));
        let truncated = truncate_width(&title, keep);
        let width = UnicodeWidthStr::width(truncated.as_str());
        (truncated, width)
    } else {
        (title, title_width)
    };
    let spacer = " ".repeat(available.saturating_sub(title_width.saturating_add(right_width)));
    let line = Line::from(vec![
        Span::styled(title, theme.title),
        Span::raw(spacer),
        Span::styled(
            right,
            if app.agent.available {
                theme.ai
            } else {
                theme.muted
            },
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn truncate_width(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    let mut output = String::new();
    let mut width = 0usize;
    let limit = max_width.saturating_sub(1);
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width + ch_width > limit {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.push('…');
    output
}

fn draw_status(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let jobs = app.job_status_line();
    let style = if app.last_error.is_some() {
        theme.err
    } else {
        theme.fg
    };
    let text = format!(" {}    {jobs}", sanitize_line(&app.status));
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
        let title = " os-int ─ Investigation: café  [TLP:AMBER] ";
        let right = " claude ● ";
        assert!(title.len() > UnicodeWidthStr::width(title));
        let title_width = UnicodeWidthStr::width(title);
        let right_width = UnicodeWidthStr::width(right);
        let available = 80usize;
        let spacer = available.saturating_sub(title_width.saturating_add(right_width));
        assert_eq!(title_width + spacer + right_width, 80);
    }
}
