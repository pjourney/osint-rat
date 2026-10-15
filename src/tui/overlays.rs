//! Modal overlays: help, command palette, search, forms, agent prompt, doctor.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::app::commands::filtered;
use crate::app::{App, Overlay};
use crate::doctor::CheckStatus;
use crate::sanitize::sanitize_line;

use super::keymap::help_text;
use super::theme::Theme;
use super::widgets::{centered, ellipsize, empty_state, input_line, list_from, pane_block};

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => text_modal(frame, area, "Help", &help_text(), app.scroll, theme),
        Overlay::Doctor => doctor(frame, area, app, theme),
        Overlay::Text {
            title,
            body,
            scroll,
        } => text_modal(frame, area, title, body, *scroll, theme),
        Overlay::Palette { query, selected } => palette(frame, area, query, *selected, theme),
        Overlay::Search { query, selected } => search(frame, area, app, query, *selected, theme),
        Overlay::AgentPrompt { input } => agent_prompt(frame, area, app, input, theme),
        Overlay::Form(form) => form_modal(frame, area, form, theme),
        Overlay::Confirm { title, message, .. } => confirm(frame, area, title, message, theme),
        Overlay::TableFilter { .. } => {}
        Overlay::ThemeCycle { original, index } => {
            theme_cycle(frame, area, original, *index, theme)
        }
    }
}

fn text_modal(frame: &mut Frame, area: Rect, title: &str, body: &str, scroll: u16, theme: &Theme) {
    let popup = centered(area, 72, 22);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(pane_block(title, true, theme)),
        popup,
    );
}

fn doctor(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines = vec![
        Line::from("Environment checks used by bug reports."),
        Line::from(""),
    ];
    for check in app.doctor_checks() {
        let style = match check.status {
            CheckStatus::Ok => theme.ok,
            CheckStatus::Missing => theme.muted,
            CheckStatus::Warn => theme.warn,
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{:<14} ", check.name)),
            Span::styled(format!("{:<8}", check.status.symbol()), style),
            Span::raw(ellipsize(&check.detail, 48, theme.glyphs.ascii)),
        ]));
    }
    let popup = centered(area, 72, 20);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Doctor", true, theme)),
        popup,
    );
}

fn palette(
    frame: &mut Frame,
    area: Rect,
    query: &crate::app::Input,
    selected: usize,
    theme: &Theme,
) {
    let commands = filtered(&query.value);
    let items: Vec<String> = commands
        .iter()
        .map(|command| format!("{:<22}  {}", command.title, command.hint))
        .collect();
    let height = (items.len() as u16 + 4).clamp(8, 18);
    let popup = centered(area, 56, height);
    frame.render_widget(Clear, popup);
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(3),
        ratatui::layout::Constraint::Min(3),
    ])
    .split(popup);
    frame.render_widget(
        Paragraph::new(input_line(query)).block(pane_block("Command palette", true, theme)),
        rows[0],
    );
    frame.render_widget(
        list_from(items, selected, theme).block(pane_block("Commands", false, theme)),
        rows[1],
    );
}

fn search(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    query: &crate::app::Input,
    selected: usize,
    theme: &Theme,
) {
    let hits = app.search_hits(&query.value);
    let items: Vec<String> = hits
        .iter()
        .map(|hit| format!("{:<8}  {}", hit.kind, hit.label))
        .collect();
    let height = (items.len() as u16 + 4).clamp(8, 20);
    let popup = centered(area, 64, height);
    frame.render_widget(Clear, popup);
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(3),
        ratatui::layout::Constraint::Min(3),
    ])
    .split(popup);
    frame.render_widget(
        Paragraph::new(input_line(query)).block(pane_block("Search investigation", true, theme)),
        rows[0],
    );
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty_state(
                "No matches",
                "Type to search pages, entities, findings, and sources.",
                "Enter opens a hit · Esc closes",
                theme,
            ))
            .block(pane_block("Results", false, theme)),
            rows[1],
        );
        return;
    }
    frame.render_widget(
        list_from(items, selected, theme).block(pane_block("Results", false, theme)),
        rows[1],
    );
}

fn agent_prompt(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    input: &crate::app::Input,
    theme: &Theme,
) {
    let popup = centered(area, 70, 8);
    frame.render_widget(Clear, popup);
    let focus = app
        .workspace
        .as_ref()
        .map(|_| {
            format!(
                "Agent: {}   page context included   Ctrl+S to send, Esc to cancel",
                app.agent.profile.name
            )
        })
        .unwrap_or_else(|| "Ask the agent".into());
    let lines = vec![
        Line::from(Span::styled(focus, theme.muted)),
        Line::from(""),
        input_line(input),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Ask agent", true, theme)),
        popup,
    );
}

fn form_modal(frame: &mut Frame, area: Rect, form: &crate::app::FormState, theme: &Theme) {
    let height = (form.fields.len() as u16 * 3 + 4).clamp(8, 20);
    let popup = centered(area, 64, height);
    frame.render_widget(Clear, popup);
    let mut lines = Vec::new();
    for (index, field) in form.fields.iter().enumerate() {
        let style = if index == form.index {
            theme.header
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{}  ({})", field.label, field.hint),
            style,
        )));
        if index == form.index {
            lines.push(input_line(&field.value));
        } else {
            lines.push(Line::from(sanitize_line(&field.value.value).into_owned()));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Ctrl+S submit   Esc cancel   Tab next field",
        theme.muted,
    )));
    let title = match form.kind {
        crate::app::FormKind::NewInvestigation => "New investigation",
        crate::app::FormKind::OpenInvestigation => "Open investigation",
        crate::app::FormKind::NewPage => "New page",
        crate::app::FormKind::NewFinding => "New finding",
        crate::app::FormKind::NewNote => "Note",
        crate::app::FormKind::SettingsAgent => "Agent settings",
        crate::app::FormKind::Ingest => "Ingest file",
        crate::app::FormKind::Playbook => "Run playbook",
        crate::app::FormKind::PlaybookPrompt => "Playbook prompt",
        crate::app::FormKind::Watch => "Watch playbook",
        crate::app::FormKind::Marking => "Set marking",
        crate::app::FormKind::TeamMember => "Add team member",
    };
    frame.render_widget(
        Paragraph::new(lines).block(pane_block(title, true, theme)),
        popup,
    );
}

fn theme_cycle(frame: &mut Frame, area: Rect, original: &str, index: usize, theme: &Theme) {
    const NAMES: [&str; 5] = ["dark", "light", "mono", "phosphor", "amber"];
    let popup = centered(area, 48, 12);
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            "Palette :theme — live preview. y saves, Esc reverts.",
            theme.muted,
        )),
        Line::from(""),
    ];
    for (i, name) in NAMES.iter().enumerate() {
        let cursor = if i == index % NAMES.len() {
            theme.glyphs.focus
        } else {
            " "
        };
        let mark = if *name == original { " (current)" } else { "" };
        let style = if i == index % NAMES.len() {
            theme.selected
        } else {
            theme.fg
        };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {name}{mark}"),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Theme", true, theme)),
        popup,
    );
}

fn confirm(frame: &mut Frame, area: Rect, title: &str, message: &str, theme: &Theme) {
    let popup = centered(area, 56, 8);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(message.to_string()),
        Line::from(""),
        Line::from(Span::styled("y confirm   Esc cancel", theme.muted)),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(pane_block(title, true, theme)),
        popup,
    );
}
