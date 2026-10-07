//! Terminal event loop and rendering. Drawing is a pure function of `App`.

mod keymap;
mod overlays;
mod screens;
mod theme;
mod widgets;

use std::time::Duration;

use color_eyre::eyre::{Result, WrapErr};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};

use crate::app::{Action, App, View};
use crate::sanitize::sanitize_line;

use self::theme::Theme;

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

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = Theme::from_config(&app.config);
    let area = frame.area();
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
    let title = format!(" os-int ─ Investigation: {investigation} ");
    let right = format!(" {agent} ");
    let spacer = " ".repeat(
        area.width
            .saturating_sub((title.len() + right.len()) as u16) as usize,
    );
    let line = Line::from(vec![
        Span::styled(title, theme.title),
        Span::raw(spacer),
        Span::styled(
            right,
            if app.agent.available {
                theme.agent
            } else {
                theme.muted
            },
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_status(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let jobs = app.job_status_line();
    let style = if app.last_error.is_some() {
        theme.error
    } else {
        theme.status
    };
    let text = format!(" {}    {jobs}", sanitize_line(&app.status));
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}
