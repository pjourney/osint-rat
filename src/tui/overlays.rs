//! Modal overlays: help, command palette, search, forms, agent prompt, doctor, proposals.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::app::commands::filtered;
use crate::app::{App, Overlay};
use crate::doctor::CheckStatus;
use crate::model::ProposalStatus;
use crate::sanitize::sanitize_line;

use super::keymap::HELP_TEXT;
use super::theme::Theme;
use super::widgets::{centered, input_line, list_from, pane_block, truncate};

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => text_modal(frame, area, "Help", HELP_TEXT, app.scroll, theme),
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
        Overlay::Proposals { run_id, selected } => {
            proposals(frame, area, app, *run_id, *selected, theme)
        }
        Overlay::Suggestions { selected } => suggestions(frame, area, app, *selected, theme),
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
            CheckStatus::Ok => theme.success,
            CheckStatus::Missing => theme.muted,
            CheckStatus::Warn => theme.warning,
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{:<14} ", check.name)),
            Span::styled(format!("{:<8}", check.status.symbol()), style),
            Span::raw(truncate(&check.detail, 48)),
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
    let list = if items.is_empty() {
        list_from(
            vec!["(type to search pages, entities, findings, sources)".into()],
            0,
            theme,
        )
    } else {
        list_from(items, selected, theme)
    };
    frame.render_widget(list.block(pane_block("Results", false, theme)), rows[1]);
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
                "Agent: {}   page context included   Enter to send, Esc to cancel",
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
        "Enter submit   Esc cancel   Tab next field",
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

fn confirm(frame: &mut Frame, area: Rect, title: &str, message: &str, theme: &Theme) {
    let popup = centered(area, 56, 8);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(message.to_string()),
        Line::from(""),
        Line::from(Span::styled("Enter confirm   Esc cancel", theme.muted)),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(pane_block(title, true, theme)),
        popup,
    );
}

fn proposals(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    run_id: uuid::Uuid,
    selected: usize,
    theme: &Theme,
) {
    let proposals: Vec<_> = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.proposals_for_run(run_id))
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(format!("{} suggests:", app.agent.profile.name)),
        Line::from(""),
    ];
    let pending: Vec<_> = proposals
        .iter()
        .filter(|proposal| proposal.status == ProposalStatus::Pending)
        .collect();
    for (index, proposal) in pending.iter().enumerate() {
        let style = if index == selected {
            theme.selected
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(proposal.action.summary(), style)));
        let cited = proposal.action.cited_sources();
        if !cited.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    evidence: {}", cited.join(", ")),
                theme.muted,
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[A] Accept   [R] Reject   Esc close",
        theme.muted,
    )));
    let popup = centered(area, 70, 16);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Agent proposals", true, theme)),
        popup,
    );
}

fn suggestions(frame: &mut Frame, area: Rect, app: &App, selected: usize, theme: &Theme) {
    let items: Vec<String> = app
        .listed_suggestions()
        .into_iter()
        .map(|suggestion| {
            let labels = suggestion
                .entity_ids
                .iter()
                .filter_map(|id| {
                    app.workspace
                        .as_ref()
                        .and_then(|workspace| workspace.entity(*id))
                        .map(|entity| entity.short_ref().to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{:>3}  {:<22}  {}  {}",
                suggestion.score,
                suggestion.rule.as_str(),
                labels,
                suggestion.reason
            )
        })
        .collect();
    let height = (items.len() as u16 + 4).clamp(8, 18);
    let popup = centered(area, 72, height);
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from("Scored suggestions. Enter proposes a merge. Nothing auto-merges."),
        Line::from(""),
    ];
    for (index, item) in items.iter().enumerate() {
        let style = if index == selected {
            theme.selected
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(item.clone(), style)));
    }
    if items.is_empty() {
        lines.push(Line::from(Span::styled("(none)", theme.muted)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter confirm   Esc close",
        theme.muted,
    )));
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Resolution suggestions", true, theme)),
        popup,
    );
}
