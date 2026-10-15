//! Terminal event loop and rendering. Drawing is a pure function of `App`.

mod glyphs;
mod keymap;
mod overlays;
mod preview;
mod screens;
mod table;
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
                        if let Some(action) = keymap::map_key(
                            key,
                            &app.overlay,
                            app.view,
                            app.is_editing(),
                            app.review_armed,
                            app.board_zoomed,
                        ) {
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
    let mut constraints = vec![Constraint::Length(1), Constraint::Min(5)];
    if app.review_armed {
        constraints.push(Constraint::Length(1));
    }
    if theme.degrade_hint.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));
    let rows = Layout::vertical(constraints).split(area);
    draw_header(frame, rows[0], app, &theme);
    if app.view == View::Welcome {
        screens::welcome(frame, rows[1], app, &theme);
    } else {
        screens::workspace(frame, rows[1], app, &theme);
    }
    let mut row = 2;
    if app.review_armed {
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
        format!("{} {jobs_active} jobs", theme.glyphs.spinner[0])
    } else {
        format!("{} jobs", theme.glyphs.done)
    };
    let title = format!(
        " os-int ▸ {investigation} ▸ {}  {marking}",
        app.view.title()
    );
    let right = format!(" {review_bit}  {jobs_bit}  {agent} ");
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
        Span::styled(right, theme.muted),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_confirm_bar(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, theme: &Theme) {
    let text = app.confirm_bar_text();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme.fg))).style(theme.panel),
        area,
    );
}

fn truncate_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
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
        assert_eq!(truncate_width("hello", 0), "");
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
}
