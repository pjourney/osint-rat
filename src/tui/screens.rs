//! Full-screen views: welcome, investigation workspace, and per-section panes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::filter::{SortKey, entity_provenances, entity_seen_at};
use crate::app::{App, EvidenceRow, InspectorTab, Overlay, Pane, View};
use crate::doctor::CheckStatus;
use crate::model::{Entity, FacetAxis, GraphEdge, GraphFilters, LinkDirection, Workspace, now};
use crate::sanitize::{sanitize_line, sanitize_text};
use crate::tools::Activity;

use super::glyphs::Glyphs;
use super::table::{Align, Column, TableCell, provenance_cells, relative_time, render_table};
use super::theme::Theme;
use super::widgets::{
    ellipsize, empty_state, error_panel, input_line, list_from, loading_state, pane_block,
    pane_title, provenance_glyph, provenance_span, rule_span,
};

pub fn welcome(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let columns =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    let mut lines = vec![
        Line::from(Span::styled("os-int", theme.title)),
        Line::from(Span::styled(
            "Open Source Intelligence Workbench",
            theme.header,
        )),
        Line::from(""),
        Line::from("Create a case, collect public evidence, accept AI in Review,"),
        Line::from("then export. AI only proposes. Every fact shows its source."),
        Line::from(""),
        welcome_item(0, app.selected, "New investigation", theme),
        welcome_item(1, app.selected, "Open investigation", theme),
        welcome_item(2, app.selected, "Doctor", theme),
        welcome_item(3, app.selected, "Help", theme),
        Line::from(""),
        Line::from(Span::styled("Recent investigations", theme.header)),
    ];
    let recent = app.investigations();
    if recent.is_empty() {
        lines.extend(empty_state(
            "No investigations yet",
            "Create a case to start collecting.",
            "n new investigation",
            theme,
        ));
    } else {
        for summary in recent.iter().take(8) {
            lines.push(Line::from(format!(
                "  {}  {}p {}s {}f",
                summary.investigation.name,
                summary.page_count,
                summary.source_count,
                summary.finding_count
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Welcome", true, theme))
            .wrap(Wrap { trim: false }),
        columns[0],
    );
    let mut detected = vec![
        Line::from(Span::styled("Detected", theme.header)),
        Line::from(""),
    ];
    for check in app.doctor_checks() {
        let style = match check.status {
            CheckStatus::Ok => theme.success,
            CheckStatus::Missing => theme.muted,
            CheckStatus::Warn => theme.warning,
        };
        detected.push(Line::from(vec![
            Span::raw(format!("  {:<14} ", check.name)),
            Span::styled(check.status.symbol(), style),
            Span::styled(
                format!("  {}", ellipsize(&check.detail, 36, theme.glyphs.ascii)),
                theme.muted,
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(detected).block(pane_block("Environment", false, theme)),
        columns[1],
    );
}

fn welcome_item<'a>(index: usize, selected: usize, label: &'a str, theme: &Theme) -> Line<'a> {
    let style = if index == selected {
        theme.selected
    } else {
        Style::default()
    };
    Line::from(Span::styled(label, style))
}

const RAIL_WIDTH: u16 = 14;
const INSPECTOR_WIDTH: u16 = 44;
pub(crate) const INSPECTOR_BREAK: u16 = 110;

/// Draws the workspace and returns the Board rect so toasts stay on it.
pub fn workspace(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) -> Rect {
    if app.board_zoomed {
        draw_main(frame, area, app, theme);
        return area;
    }
    if app.is_plain_linear() {
        return workspace_linear(frame, area, app, theme);
    }
    if area.width < INSPECTOR_BREAK {
        let target = (area.height as u32 * 40 / 100).max(6) as u16;
        let drawer = app.motion.drawer_height(target);
        let rows = Layout::vertical([Constraint::Min(6), Constraint::Length(drawer)]).split(area);
        let rail_width = if area.width < 80 { 3 } else { RAIL_WIDTH };
        let columns = Layout::horizontal([
            Constraint::Length(rail_width),
            Constraint::Length(1),
            Constraint::Min(8),
        ])
        .split(rows[0]);
        draw_rail(frame, columns[0], app, theme, area.width < 80);
        frame.render_widget(Paragraph::new(rule_span(theme)), columns[1]);
        draw_main(frame, columns[2], app, theme);
        draw_inspector(frame, rows[1], app, theme);
        return columns[2];
    }
    let columns = Layout::horizontal([
        Constraint::Length(RAIL_WIDTH),
        Constraint::Length(1),
        Constraint::Min(20),
        Constraint::Length(1),
        Constraint::Length(INSPECTOR_WIDTH),
    ])
    .split(area);
    draw_rail(frame, columns[0], app, theme, false);
    frame.render_widget(Paragraph::new(rule_span(theme)), columns[1]);
    draw_main(frame, columns[2], app, theme);
    frame.render_widget(Paragraph::new(rule_span(theme)), columns[3]);
    draw_inspector(frame, columns[4], app, theme);
    columns[2]
}

fn workspace_linear(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) -> Rect {
    let rail_height: u16 = 2;
    let min_board = 10u16;
    let preferred_inspector = ((area.height as u32 * 30) / 100).max(6) as u16;
    let inspector_height = preferred_inspector
        .min(
            area.height
                .saturating_sub(rail_height.saturating_add(min_board)),
        )
        .max(4)
        .min(area.height.saturating_sub(rail_height.saturating_add(6)));
    let rows = Layout::vertical([
        Constraint::Length(rail_height.min(area.height)),
        Constraint::Min(6),
        Constraint::Length(inspector_height),
    ])
    .split(area);
    draw_rail_linear(frame, rows[0], app, theme);
    draw_main(frame, rows[1], app, theme);
    draw_inspector(frame, rows[2], app, theme);
    rows[1]
}

fn draw_rail_linear(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let review = app.pending_review_count();
    let mut spans = Vec::new();
    for (index, (label, view)) in app.nav_items().into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let current = app.rail_view_is_current(view);
        let mut text = format!("{} {label}", index + 1);
        if view == View::Review && review > 0 {
            text = format!("{text} {review}");
        }
        if current {
            text = format!("{}{text}", theme.glyphs.focus);
        }
        let style = if current { theme.selected } else { theme.fg };
        spans.push(Span::styled(text, style));
    }
    let lines = vec![
        Line::from(Span::styled("RAIL", theme.header)),
        Line::from(spans),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_rail(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, digits_only: bool) {
    let focused = app.pane == Pane::Nav;
    let review = app.pending_review_count();
    let jobs = app
        .job_records
        .values()
        .filter(|job| job.status.is_active())
        .count();
    let pulse = if focused {
        app.motion.frame_kind(super::motion::AnimKind::Focus)
    } else {
        None
    };
    let mut lines = vec![super::widgets::pane_title_pulse(
        "Rail", focused, theme, pulse,
    )];
    for (index, (label, view)) in app.nav_items().into_iter().enumerate() {
        let current = app.rail_view_is_current(view);
        let mut text = if digits_only {
            format!("{}", index + 1)
        } else {
            format!("{} {label}", index + 1)
        };
        if view == View::Review && review > 0 {
            text = format!("{text} {review}");
        }
        if view == View::Jobs && jobs > 0 {
            text = format!("{text} {}", app.motion.job_spinner_glyph(&theme.glyphs));
        }
        let style = if current {
            theme.accent.add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            theme.fg
        };
        let prefix = if current { theme.glyphs.focus } else { " " };
        lines.push(Line::from(Span::styled(format!("{prefix}{text}"), style)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_main(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focused = app.pane == Pane::Main;
    match app.view {
        View::Welcome => {}
        View::Overview => overview(frame, area, app, theme, focused),
        View::Evidence => evidence(frame, area, app, theme, focused),
        View::Review => review(frame, area, app, theme, focused),
        View::Pages => pages(frame, area, app, theme, focused),
        View::Sources => sources(frame, area, app, theme, focused),
        View::Findings => findings(frame, area, app, theme, focused),
        View::Entities => entities(frame, area, app, theme, focused),
        View::Timeline => timeline(frame, area, app, theme, focused),
        View::Notes => notes(frame, area, app, theme, focused),
        View::Artifacts => artifacts(frame, area, app, theme, focused),
        View::Agent => agent(frame, area, app, theme, focused),
        View::Jobs => jobs(frame, area, app, theme, focused),
        View::Tools => tools(frame, area, app, theme, focused),
        View::Settings => settings(frame, area, app, theme, focused),
        View::Object => object_view(frame, area, app, theme, focused),
        View::Graph => graph(frame, area, app, theme, focused),
        View::Audit => audit(frame, area, app, theme, focused),
    }
}

fn overview(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let home = app.case_home();
    let mut lines = vec![pane_title("TARGETS", focused, theme)];
    if let Some(message) = app.last_error.as_deref() {
        lines.extend(error_panel(message, theme));
    }
    if home.is_empty {
        lines.extend(empty_state(
            "No entities yet",
            "Collect on a target to populate this case.",
            &["c collect", "n new target"].join(theme.glyphs.sep()),
            theme,
        ));
    } else if home.targets.is_empty() {
        lines.extend(empty_state(
            "No targets yet",
            "Add a domain, IP, URL, or repository to collect.",
            "n new target",
            theme,
        ));
    } else {
        for (index, target) in home.targets.iter().enumerate() {
            let cursor = if index == app.selected {
                theme.glyphs.focus
            } else {
                " "
            };
            let glyph = target
                .provenance
                .map(|provenance| provenance_glyph(provenance, theme))
                .unwrap_or_else(|| Span::styled(theme.glyphs.dash(), theme.muted));
            let src = if target.source_count == 0 {
                theme.glyphs.dash().to_string()
            } else {
                target.source_count.to_string()
            };
            let mut status = App::case_target_status_label(target, theme.glyphs.ascii);
            if target.never_collected {
                status = format!("{status}  c collect");
            }
            let done = if target.running {
                if app.motion.mode() == super::motion::MotionMode::Off || app.is_plain_linear() {
                    "running"
                } else {
                    app.motion.job_spinner_glyph(&theme.glyphs)
                }
            } else if target.never_collected {
                ""
            } else if theme.glyphs.ascii {
                "ok"
            } else {
                "✓"
            };
            let style = if index == app.selected {
                theme.selected
            } else {
                theme.fg
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{cursor}{} ", target.kind_tag()), style),
                Span::styled(
                    format!("{:<22}", ellipsize(&target.value, 22, theme.glyphs.ascii)),
                    style,
                ),
                glyph,
                Span::styled(format!(" {src:<3}  {status} {done}"), theme.muted),
            ]));
            if target.running {
                lines.extend(observing_radar_lines(app, theme));
            }
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("AT A GLANCE", theme.header)));
    lines.push(glance_line("entities", home.entities, theme));
    lines.push(glance_line("findings", home.findings, theme));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("NEXT", theme.header)));
    if home.next.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Case is current. Ctrl+K for commands.",
            theme.muted,
        )));
    }
    for item in &home.next {
        let mark = if item.hint.contains("review") {
            theme.glyphs.ai
        } else {
            "!"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} {}  ", item.label), theme.fg),
            Span::styled(item.hint.clone(), theme.accent),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

fn observing_radar_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    super::motion::radar_lines(
        app.motion
            .frame_kind(super::motion::AnimKind::Radar)
            .unwrap_or_else(|| super::motion::FrameView::first(8)),
        theme,
        theme.glyphs.ascii,
        app.motion.mode(),
    )
}

fn glance_line(
    label: &str,
    counts: crate::app::case::GlanceCounts,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::raw(format!("  {:>2} {label:<9}  ", counts.total))];
    let mut wrote = false;
    for (provenance, count) in counts.nonzero_buckets() {
        if wrote {
            spans.push(Span::raw("  "));
        }
        wrote = true;
        spans.push(provenance_glyph(provenance, theme));
        spans.push(Span::raw(format!(" {count}")));
    }
    if !wrote {
        spans.push(Span::styled("none", theme.muted));
    }
    Line::from(spans)
}

fn evidence(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    if app.time_lens {
        timeline(frame, area, app, theme, focused);
        return;
    }
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let listed = app.listed_evidence();
    let columns = [
        Column {
            key: SortKey::Kind,
            header: "KIND",
            min: 4,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Value,
            header: "VALUE",
            min: 8,
            weight: 1,
            align: Align::Left,
        },
        Column {
            key: SortKey::Prov,
            header: "PROV",
            min: 4,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Src,
            header: "SRC",
            min: 3,
            weight: 0,
            align: Align::Right,
        },
        Column {
            key: SortKey::Seen,
            header: "SEEN",
            min: 5,
            weight: 0,
            align: Align::Right,
        },
    ];
    let mut rows = Vec::new();
    for row in &listed {
        match row {
            EvidenceRow::Finding(id) => {
                let Some(finding) = workspace.finding(*id) else {
                    continue;
                };
                rows.push(vec![
                    TableCell::text("F"),
                    TableCell::text(finding.title.clone()),
                    provenance_cells(&[finding.provenance], theme),
                    TableCell::text(finding.source_ids.len().to_string()),
                    TableCell::text(relative_time(finding.updated_at)),
                ]);
            }
            EvidenceRow::Source(id) => {
                let Some(source) = workspace.source(*id) else {
                    continue;
                };
                let seen = source.last_seen_at.unwrap_or(source.retrieved_at);
                rows.push(vec![
                    TableCell::text("S"),
                    TableCell::text(source.title.clone()),
                    provenance_cells(&[source.provenance], theme),
                    TableCell::text("1"),
                    TableCell::text(relative_time(seen)),
                ]);
            }
        }
    }
    let empty = table_empty(
        app,
        theme,
        "No evidence yet",
        "Collect on a target to populate this case.",
        &["c collect", "n new target"].join(theme.glyphs.sep()),
    );
    draw_table_board(
        frame,
        area,
        "EVIDENCE",
        &columns,
        &rows,
        app,
        focused,
        theme,
        listed.len(),
        app.unfiltered_evidence_count(),
        empty,
    );
}

fn review(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items = app.listed_review_items();
    let mut lines = vec![
        pane_title("AI PROPOSES ONLY", focused, theme),
        Line::from(Span::styled(
            "nothing is written until you accept",
            theme.muted,
        )),
    ];
    if let Some(message) = app.last_error.as_deref() {
        lines.extend(error_panel(message, theme));
    }
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "No pending AI, playbook, or merge proposals.",
            theme.muted,
        )));
    }
    let mut last_group = String::new();
    for (index, item) in items.iter().enumerate() {
        if item.group != last_group {
            lines.push(Line::from(Span::styled(item.group.clone(), theme.header)));
            last_group = item.group.clone();
        }
        let mark = if index == app.selected {
            theme.glyphs.focus
        } else if app.review_marks.contains(&index) {
            "x"
        } else {
            " "
        };
        let glyph = if item.is_ai() {
            theme.glyphs.ai_pending
        } else {
            theme.glyphs.analyst
        };
        let style = if index == app.selected {
            theme.selected
        } else {
            theme.fg
        };
        lines.push(Line::from(Span::styled(
            format!("{mark}{glyph} {}", item.title),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_board_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<String>,
    selected: usize,
    focused: bool,
    theme: &Theme,
    app: &App,
) {
    let error = app
        .last_error
        .as_deref()
        .map(|message| error_panel(message, theme));
    let header = if error.is_some() { 3 } else { 1 };
    let rows = Layout::vertical([Constraint::Length(header), Constraint::Min(1)]).split(area);
    let mut heading = vec![pane_title(title, focused, theme)];
    if let Some(error) = error {
        heading.extend(error);
    }
    frame.render_widget(Paragraph::new(heading), rows[0]);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty_state(
                &format!("No {title} yet"),
                "Nothing to show on this board.",
                &["n new", "c collect"].join(theme.glyphs.sep()),
                theme,
            )),
            rows[1],
        );
        return;
    }
    frame.render_widget(list_from(items, selected, theme), rows[1]);
}

fn table_empty(
    app: &App,
    theme: &Theme,
    title: &str,
    body: &str,
    actions: &str,
) -> Vec<Line<'static>> {
    if !app.active_board_filter().is_empty() {
        empty_state(
            "No matching rows",
            "Nothing matches this filter.",
            "x clear filter",
            theme,
        )
    } else {
        empty_state(title, body, actions, theme)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_table_board(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    columns: &[Column],
    rows: &[Vec<TableCell>],
    app: &App,
    focused: bool,
    theme: &Theme,
    shown: usize,
    total: usize,
    empty: Vec<Line<'static>>,
) {
    let filter = app.active_board_filter();
    let editing = matches!(app.overlay, Overlay::TableFilter { .. });
    let error = app
        .last_error
        .as_deref()
        .map(|message| error_panel(message, theme));
    let mut header = 1u16;
    if editing || !filter.is_empty() {
        header += 1;
    }
    if error.is_some() {
        header += 2;
    }
    let split = Layout::vertical([Constraint::Length(header), Constraint::Min(1)]).split(area);
    let mut lines = vec![pane_title(title, focused, theme)];
    if editing || !filter.is_empty() {
        if let Overlay::TableFilter { query } = &app.overlay {
            let mut filter_line = vec![Span::styled("/", theme.accent)];
            filter_line.extend(input_line(query).spans);
            filter_line.push(Span::raw("  "));
            filter_line.push(Span::styled(
                format!("{total} {} {shown}", theme.glyphs.arrow()),
                theme.muted,
            ));
            lines.push(Line::from(filter_line));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("/{filter}"), theme.accent),
                Span::raw("  "),
                Span::styled(
                    format!("{total} {} {shown}", theme.glyphs.arrow()),
                    theme.muted,
                ),
            ]));
        }
    }
    if let Some(error) = error {
        lines.extend(error);
    }
    frame.render_widget(Paragraph::new(lines), split[0]);
    if rows.is_empty() {
        if let Some((job_ref, label)) = loading_job(app) {
            let lines = if app.motion.mode() == super::motion::MotionMode::Off {
                loading_state(&job_ref, &label, theme)
            } else {
                super::widgets::loading_state_spin(
                    &job_ref,
                    &label,
                    theme,
                    app.motion.job_spinner_glyph(&theme.glyphs),
                )
            };
            frame.render_widget(Paragraph::new(lines), split[1]);
        } else {
            frame.render_widget(Paragraph::new(empty), split[1]);
        }
        return;
    }
    render_table(
        frame,
        split[1],
        columns,
        rows,
        app.selected,
        app.effective_sort(),
        focused,
        theme,
    );
}

fn loading_job(app: &App) -> Option<(String, String)> {
    app.job_records
        .values()
        .find(|job| job.status.is_active())
        .map(|job| (job.short_ref().to_string(), job.label.clone()))
}

#[allow(clippy::too_many_arguments)]
fn render_board_lines(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    mut lines: Vec<Line<'static>>,
    focused: bool,
    theme: &Theme,
    app: &App,
    scroll: u16,
) {
    lines.insert(0, pane_title(title, focused, theme));
    if let Some(message) = app.last_error.as_deref() {
        lines.splice(1..1, error_panel(message, theme));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

fn object_view(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(view) = app.current_object_view() else {
        render_board_lines(
            frame,
            area,
            "Object",
            vec![Line::from("Select an entity or page, then press Enter.")],
            focused,
            theme,
            app,
            0,
        );
        return;
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{}  ", view.short_ref), theme.muted),
            Span::styled(view.title.clone(), theme.title),
            Span::raw(format!("  {}", view.object_label)),
        ]),
        Line::from(Span::styled(
            "Properties, catalogued links, sources, and findings. Esc returns.",
            theme.muted,
        )),
    ];
    for group in &view.groups {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(group.label.clone(), theme.header)));
        for property in &group.properties {
            lines.push(Line::from(format!(
                "  {:<16}  {}",
                property.label,
                ellipsize(&property.value, 56, theme.glyphs.ascii)
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Linked objects", theme.header)));
    if view.links.is_empty() {
        lines.push(Line::from(Span::styled("  None yet.", theme.muted)));
    }
    for group in &view.links {
        let suffix = if group.catalogued { "" } else { "  (other)" };
        lines.push(Line::from(Span::styled(
            format!("  {}{suffix}", group.label),
            theme.header,
        )));
        for link in &group.links {
            let arrow = match link.direction {
                LinkDirection::Outgoing => format!("--{}-->", link.relationship_kind),
                LinkDirection::Incoming => format!("<--{}--", link.relationship_kind),
            };
            lines.push(Line::from(format!(
                "    {arrow}  {}  {}  {}",
                link.entity_ref, link.entity_kind, link.entity_label
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Sources", theme.header)));
    if view.sources.is_empty() {
        lines.push(Line::from(Span::styled("  None yet.", theme.muted)));
    }
    for source in &view.sources {
        lines.push(Line::from(format!(
            "  {}  [{}]  {}",
            source.reference,
            source.provenance,
            ellipsize(&source.title, 48, theme.glyphs.ascii)
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Findings", theme.header)));
    if view.findings.is_empty() {
        lines.push(Line::from(Span::styled("  None yet.", theme.muted)));
    }
    for finding in &view.findings {
        lines.push(Line::from(format!(
            "  {}  [{}]  {}",
            finding.reference,
            finding.provenance,
            ellipsize(&finding.title, 48, theme.glyphs.ascii)
        )));
    }
    if !view.constituents.is_empty() {
        lines.push(Line::from(""));
        let heading = match &view.resolution_ref {
            Some(reference) => format!("Constituents ({reference})"),
            None => "Constituents".into(),
        };
        lines.push(Line::from(Span::styled(heading, theme.header)));
        for constituent in &view.constituents {
            lines.push(Line::from(format!(
                "  {}  {:<10}  {}",
                constituent.reference,
                constituent.kind,
                ellipsize(&constituent.label, 40, theme.glyphs.ascii)
            )));
        }
        lines.push(Line::from(Span::styled(
            "  palette: Unresolve entity",
            theme.muted,
        )));
    }
    render_board_lines(
        frame, area, "Object", lines, focused, theme, app, app.scroll,
    );
}

fn graph(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    if app.time_lens {
        timeline(frame, area, app, theme, focused);
        return;
    }
    let visible = app.visible_graph();
    let mut lines = vec![pane_title("Graph", focused, theme)];
    lines.push(Line::from(Span::styled(
        graph_title(app, theme.glyphs.ascii),
        theme.header,
    )));
    lines.push(graph_kind_counts_line(app, &visible, theme));
    if !app.graph_facet_chips().is_empty() {
        lines.push(graph_facet_line(app, &app.graph_facet_chips(), theme));
    }
    lines.push(Line::from(Span::styled(
        graph_filter_line(&app.graph.filters, theme.glyphs.ascii),
        theme.muted,
    )));
    if visible.nodes.is_empty() {
        lines.extend(empty_state(
            "No graph yet",
            "Collect on a target, then press g or s.",
            &["c collect", "n new target", "g graph"].join(theme.glyphs.sep()),
            theme,
        ));
    } else {
        lines.extend(graph_canvas_lines(app, &visible, theme));
    }
    if let Some(message) = app.last_error.as_deref() {
        lines.splice(1..1, error_panel(message, theme));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

fn graph_kind_counts_line(
    app: &App,
    visible: &crate::model::VisibleGraph,
    theme: &Theme,
) -> Line<'static> {
    let Some(workspace) = app.workspace.as_ref() else {
        return Line::from(Span::styled("KIND", theme.muted));
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for node in &visible.nodes {
        let Some(entity) = workspace.entity(node.id) else {
            continue;
        };
        let tag = Glyphs::entity_tag(&entity.kind).trim().to_string();
        if let Some((_, count)) = counts.iter_mut().find(|(name, _)| *name == tag) {
            *count += 1;
        } else {
            counts.push((tag, 1));
        }
    }
    let mut spans = vec![Span::styled("KIND  ", theme.muted)];
    for (index, (tag, count)) in counts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::raw(format!("{tag} {count}")));
    }
    if spans.len() == 1 {
        spans.push(Span::styled("none", theme.muted));
    }
    Line::from(spans)
}

fn graph_title(app: &App, ascii: bool) -> String {
    let result = &app.graph.result;
    let sep = super::glyphs::chrome_sep(ascii);
    let cap = if result.truncated {
        format!("{sep}capped")
    } else {
        String::new()
    };
    format!(
        "{} hop(s){sep}{} nodes{sep}{} edges{sep}cap {}/{}{cap}",
        app.graph.hops,
        result.nodes.len(),
        result.edges.len(),
        result.node_cap,
        result.edge_cap
    )
}

fn graph_facet_line(app: &App, chips: &[crate::model::FacetChip], theme: &Theme) -> Line<'static> {
    if chips.is_empty() {
        return Line::from(Span::styled("No facets yet.", theme.muted));
    }
    let mut spans = Vec::new();
    let mut last_axis = None;
    for (index, chip) in chips.iter().enumerate() {
        if last_axis.is_some() && last_axis != Some(chip.axis) {
            let sep = if theme.glyphs.ascii { " | " } else { " │ " };
            spans.push(Span::styled(sep, theme.muted));
        } else if index > 0 {
            spans.push(Span::raw("  "));
        }
        last_axis = Some(chip.axis);
        let active = match chip.axis {
            FacetAxis::Kind => app.graph.filters.kind.as_deref() == Some(chip.key.as_str()),
            FacetAxis::Link => app.graph.filters.link_type.as_deref() == Some(chip.key.as_str()),
            FacetAxis::Provenance => app
                .graph
                .filters
                .provenance
                .is_some_and(|provenance| provenance.as_str() == chip.key),
        };
        let cursor = index == app.graph.facet_index;
        let label = format!("{}:{}", chip.label, chip.count);
        let style = if cursor {
            theme.selected
        } else if active {
            theme.header
        } else {
            Style::default()
        };
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}

fn graph_filter_line(filters: &GraphFilters, ascii: bool) -> String {
    if filters.is_empty() {
        return "x clear   +/- hops   s Search Around   t time lens".into();
    }
    let mut parts = Vec::new();
    if let Some(kind) = &filters.kind {
        parts.push(format!("kind={kind}"));
    }
    if let Some(link) = &filters.link_type {
        parts.push(format!("link={link}"));
    }
    if let Some(provenance) = filters.provenance {
        parts.push(format!("prov={}", provenance.badge()));
    }
    format!(
        "Filter {}   x clear",
        parts.join(super::glyphs::chrome_sep(ascii))
    )
}

fn graph_canvas_lines(
    app: &App,
    visible: &crate::model::VisibleGraph,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if visible.nodes.is_empty() {
        return vec![Line::from(
            "No nodes. Collect on a domain page, then press g or s.".to_string(),
        )];
    }
    graph_tree_lines(app, visible, theme)
}

struct GraphTreeCtx<'a> {
    workspace: &'a Workspace,
    theme: &'a Theme,
    hop_of: &'a std::collections::HashMap<uuid::Uuid, u32>,
    adjacency: &'a std::collections::HashMap<uuid::Uuid, Vec<&'a GraphEdge>>,
    plus: &'a std::collections::HashSet<uuid::Uuid>,
    selected: Option<uuid::Uuid>,
    revealed: u32,
    edges_only: bool,
}

fn graph_tree_lines(
    app: &App,
    visible: &crate::model::VisibleGraph,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(workspace) = app.workspace.as_ref() else {
        return vec![Line::from("No investigation.".to_string())];
    };
    let plus: std::collections::HashSet<uuid::Uuid> = app.graph.plus_ids.iter().copied().collect();
    let hop_of: std::collections::HashMap<uuid::Uuid, u32> = visible
        .nodes
        .iter()
        .map(|node| (node.id, node.hop))
        .collect();
    let selected = app.graph_tree_ids().get(app.graph.selected).copied();
    let mut adjacency: std::collections::HashMap<uuid::Uuid, Vec<&GraphEdge>> =
        std::collections::HashMap::new();
    for edge in &visible.edges {
        adjacency.entry(edge.from_id).or_default().push(edge);
        adjacency.entry(edge.to_id).or_default().push(edge);
    }
    let ctx = GraphTreeCtx {
        workspace,
        theme,
        hop_of: &hop_of,
        adjacency: &adjacency,
        plus: &plus,
        selected,
        revealed: u32::from(app.motion.graph_revealed_hop()),
        edges_only: app.motion.graph_edges_only(),
    };
    let mut roots: Vec<uuid::Uuid> = Vec::new();
    if let Some(focus) = app.graph.focus_id.filter(|id| hop_of.contains_key(id)) {
        roots.push(focus);
    }
    if roots.is_empty() {
        roots = app
            .graph
            .result
            .seed_ids
            .iter()
            .copied()
            .filter(|id| hop_of.contains_key(id))
            .collect();
    }
    if roots.is_empty() {
        roots = visible
            .nodes
            .iter()
            .filter(|node| node.hop == 0)
            .map(|node| node.id)
            .collect();
    }
    if roots.is_empty()
        && let Some(id) = selected
    {
        roots.push(id);
    }
    let mut lines = Vec::new();
    let mut drawn = std::collections::HashSet::new();
    for root in &roots {
        push_graph_tree_node(&mut lines, &ctx, &mut drawn, *root, None, "", true);
    }
    let hidden = visible
        .nodes
        .iter()
        .filter(|node| node.hop > ctx.revealed || !drawn.contains(&node.id))
        .count();
    if hidden > 0 {
        lines.push(Line::from(Span::styled(
            format!(" +{hidden} more{}+ expand", theme.glyphs.sep()),
            theme.muted,
        )));
    }
    lines
}

fn push_graph_tree_node(
    lines: &mut Vec<Line<'static>>,
    ctx: &GraphTreeCtx<'_>,
    drawn: &mut std::collections::HashSet<uuid::Uuid>,
    id: uuid::Uuid,
    parent: Option<uuid::Uuid>,
    prefix: &str,
    is_root: bool,
) {
    let hop = ctx.hop_of.get(&id).copied().unwrap_or(0);
    if hop > ctx.revealed {
        return;
    }
    let already = !drawn.insert(id);
    if is_root {
        lines.push(graph_tree_node_line(ctx, id, already));
    }
    if already && !is_root {
        return;
    }
    let mut children: Vec<(uuid::Uuid, &GraphEdge, u32)> = ctx
        .adjacency
        .get(&id)
        .into_iter()
        .flatten()
        .filter_map(|edge| {
            let other = if edge.from_id == id {
                edge.to_id
            } else {
                edge.from_id
            };
            if other == id || Some(other) == parent {
                return None;
            }
            let child_hop = ctx
                .hop_of
                .get(&other)
                .copied()
                .unwrap_or(hop.saturating_add(1));
            if child_hop > ctx.revealed {
                return None;
            }
            Some((other, *edge, child_hop))
        })
        .collect();
    children.sort_by(|left, right| {
        left.2
            .cmp(&right.2)
            .then(left.1.kind.cmp(&right.1.kind))
            .then(left.0.cmp(&right.0))
    });
    children.dedup_by(|left, right| left.0 == right.0);
    let count = children.len();
    for (index, (child, edge, child_hop)) in children.into_iter().enumerate() {
        let last = index + 1 == count;
        let branch = graph_tree_branch(ctx.theme.glyphs.ascii, last);
        let next_prefix = format!("{prefix}{}", graph_tree_pad(ctx.theme.glyphs.ascii, last));
        let seen = drawn.contains(&child);
        let hide_label = ctx.edges_only && child_hop == ctx.revealed && child_hop > hop && !seen;
        lines.push(graph_tree_edge_line(
            ctx, prefix, branch, edge, child, seen, hide_label,
        ));
        if seen || hide_label {
            continue;
        }
        push_graph_tree_node(lines, ctx, drawn, child, Some(id), &next_prefix, false);
    }
}

fn graph_tree_branch(ascii: bool, last: bool) -> &'static str {
    if ascii {
        if last { "`- " } else { "|- " }
    } else if last {
        "└─ "
    } else {
        "├─ "
    }
}

fn graph_tree_pad(ascii: bool, last_child: bool) -> &'static str {
    if last_child {
        "   "
    } else if ascii {
        "|  "
    } else {
        "│  "
    }
}

fn graph_tree_node_line(ctx: &GraphTreeCtx<'_>, id: uuid::Uuid, seen: bool) -> Line<'static> {
    let Some(entity) = ctx.workspace.entity(id) else {
        return Line::from("(missing entity)".to_string());
    };
    let cursor = if ctx.selected == Some(id) {
        ctx.theme.glyphs.focus
    } else {
        " "
    };
    let plus_mark = if ctx.plus.contains(&id) { " +" } else { "" };
    let seen_mark = if seen { " (↑ seen)" } else { "" };
    Line::from(vec![
        Span::styled(cursor.to_string(), ctx.theme.accent),
        Span::raw(format!(
            "{} {}",
            super::glyphs::Glyphs::entity_tag(&entity.kind),
            ellipsize(entity.display_name(), 28, ctx.theme.glyphs.ascii)
        )),
        Span::raw(" "),
        provenance_glyph(entity.provenance, ctx.theme),
        Span::styled(format!("{plus_mark}{seen_mark}"), ctx.theme.accent),
    ])
}

fn graph_tree_edge_line(
    ctx: &GraphTreeCtx<'_>,
    prefix: &str,
    branch: &str,
    edge: &GraphEdge,
    child: uuid::Uuid,
    seen: bool,
    hide_label: bool,
) -> Line<'static> {
    let sources = ctx
        .workspace
        .data
        .relationships
        .iter()
        .find(|relationship| relationship.id == edge.id)
        .map(|relationship| relationship.source_ids.len().max(1))
        .unwrap_or(1);
    let stroke = super::motion::corroboration_stroke(sources, ctx.theme.glyphs.ascii);
    let arrow = if hide_label {
        String::new()
    } else {
        ctx.theme.glyphs.edge_out.to_string()
    };
    let mut spans = vec![
        Span::raw(format!("{prefix}{branch}")),
        Span::styled(
            format!("{} {stroke}{arrow} ", edge.kind),
            ctx.theme.corroboration_style(sources),
        ),
    ];
    if hide_label {
        return Line::from(spans);
    }
    let Some(entity) = ctx.workspace.entity(child) else {
        spans.push(Span::raw("(missing)".to_string()));
        return Line::from(spans);
    };
    let cursor = if ctx.selected == Some(child) {
        ctx.theme.glyphs.focus
    } else {
        ""
    };
    let plus_mark = if ctx.plus.contains(&child) { " +" } else { "" };
    let seen_mark = if seen {
        if ctx.theme.glyphs.ascii {
            " (^ seen)"
        } else {
            " (↑ seen)"
        }
    } else {
        ""
    };
    spans.push(Span::styled(cursor.to_string(), ctx.theme.accent));
    spans.push(Span::raw(format!(
        "{} {}",
        super::glyphs::Glyphs::entity_tag(&entity.kind),
        ellipsize(entity.display_name(), 22, ctx.theme.glyphs.ascii)
    )));
    spans.push(Span::raw(" "));
    spans.push(provenance_glyph(entity.provenance, ctx.theme));
    spans.push(Span::styled(
        format!("{plus_mark}{seen_mark}"),
        ctx.theme.accent,
    ));
    Line::from(spans)
}

fn pages(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items = app
        .workspace
        .as_ref()
        .map(|workspace| {
            workspace
                .data
                .pages
                .iter()
                .map(|page| {
                    format!(
                        "{}  {:<12}  {}{}",
                        page.short_ref(),
                        page.kind.label(),
                        page.title,
                        page.target
                            .as_deref()
                            .map(|target| format!("  ({target})"))
                            .unwrap_or_default()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    render_board_list(
        frame,
        area,
        "Pages",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn sources(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items = app
        .workspace
        .as_ref()
        .map(|workspace| {
            workspace
                .data
                .sources
                .iter()
                .map(|source| {
                    format!(
                        "{}  {:<8}  {}  {}",
                        source.short_ref(),
                        source.kind.label(),
                        source.provenance.badge(),
                        source.title
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    render_board_list(
        frame,
        area,
        "Sources",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn findings(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let items: Vec<String> = workspace
        .data
        .findings
        .iter()
        .map(|finding| {
            format!(
                "{}  {}  {}  [{}]  {}",
                finding.short_ref(),
                finding.provenance.badge(),
                finding.confidence.as_str(),
                workspace.format_source_refs(&finding.source_ids),
                finding.title
            )
        })
        .collect();
    render_board_list(
        frame,
        area,
        "Findings",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn entities(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    if app.time_lens {
        timeline(frame, area, app, theme, focused);
        return;
    }
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let listed = app.listed_entities();
    let columns = [
        Column {
            key: SortKey::Kind,
            header: "KIND",
            min: 4,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Value,
            header: "VALUE",
            min: 8,
            weight: 1,
            align: Align::Left,
        },
        Column {
            key: SortKey::Prov,
            header: "PROV",
            min: 4,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Src,
            header: "SRC",
            min: 3,
            weight: 0,
            align: Align::Right,
        },
        Column {
            key: SortKey::Seen,
            header: "SEEN",
            min: 5,
            weight: 0,
            align: Align::Right,
        },
    ];
    let mut rows = Vec::new();
    for entity in &listed {
        let provenances = entity_provenances(entity, workspace);
        let seen = entity_seen_at(entity, workspace)
            .map(relative_time)
            .unwrap_or_else(|| "never".into());
        let mark = if app.merge_marks.contains(&entity.id) {
            "x"
        } else {
            " "
        };
        rows.push(vec![
            TableCell::text(format!("{mark}{}", Glyphs::entity_tag(&entity.kind))),
            TableCell::text(entity.display_name().to_string()),
            provenance_cells(&provenances, theme),
            TableCell::text(entity.source_ids.len().to_string()),
            TableCell::text(seen),
        ]);
    }
    let empty = table_empty(
        app,
        theme,
        "No entities yet",
        "Collect on a target to populate this case.",
        &["c collect", "n new target"].join(theme.glyphs.sep()),
    );
    draw_table_board(
        frame,
        area,
        "ENTITIES",
        &columns,
        &rows,
        app,
        focused,
        theme,
        listed.len(),
        app.unfiltered_entity_count(),
        empty,
    );
}

fn audit(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let mut items: Vec<String> = Vec::new();
    for entry in &workspace.data.audit {
        items.push(format!(
            "{}  {}  {}@{}  {}",
            entry.short_ref(),
            entry.kind.label(),
            entry.actor,
            entry.machine,
            ellipsize(&entry.summary, 48, theme.glyphs.ascii)
        ));
    }
    render_board_list(
        frame,
        area,
        "Audit",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn timeline(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(area);
    let filter = match app.timeline.filter_entity_id.and_then(|id| {
        app.workspace
            .as_ref()
            .and_then(|workspace| workspace.entity(id))
            .map(|entity| entity.display_name().to_string())
    }) {
        Some(name) => format!("filter {name}"),
        None => "all entities".into(),
    };
    let mode = if app.timeline.nearby {
        "nearby"
    } else {
        "overlay"
    };
    let header = vec![
        pane_title("Timeline", focused, theme),
        Line::from(Span::styled(
            format!(
                "Time lens {mode}{}{filter}{}x clear",
                theme.glyphs.sep(),
                theme.glyphs.sep()
            ),
            theme.header,
        )),
        Line::from(Span::styled(
            "Collection log + imported Events. Enter opens a linked entity. Esc restores.",
            theme.muted,
        )),
    ];
    frame.render_widget(Paragraph::new(header), rows[0]);
    let items: Vec<String> = if app.timeline.nearby {
        app.listed_nearby()
            .iter()
            .map(|entity| {
                format!(
                    "{}  {:<12}  {}  {}",
                    entity.short_ref(),
                    entity.kind.label(),
                    entity.fields.get("country").unwrap_or("-"),
                    entity.display_name()
                )
            })
            .collect()
    } else {
        app.listed_timeline_rows()
            .iter()
            .map(|event| {
                let end = event
                    .ended_at
                    .map(|ended| format!("{}{}", theme.glyphs.arrow(), ended.format("%H:%M")))
                    .unwrap_or_default();
                format!(
                    "{}  {}{end}  {:<16}  {}",
                    event.short_ref,
                    event.started_at.format("%Y-%m-%d"),
                    event.kind.label(),
                    event.title
                )
            })
            .collect()
    };
    let selected = app.selected.min(items.len().saturating_sub(1));
    render_board_list(
        frame, rows[1], "Events", items, selected, focused, theme, app,
    );
}

fn notes(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let mut items: Vec<String> = Vec::new();
    if let Some(page) = app.page_id.and_then(|id| workspace.page(id))
        && !page.notes.trim().is_empty()
    {
        items.push(format!(
            "{} page notes  {}",
            page.short_ref(),
            ellipsize(&page.notes, 60, theme.glyphs.ascii)
        ));
    }
    for note in &workspace.data.notes {
        items.push(format!(
            "{}  {}  {}",
            note.short_ref(),
            note.provenance.badge(),
            note.headline()
        ));
    }
    render_board_list(
        frame,
        area,
        "Notes",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn agent(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let run = app
        .agent_run_id
        .and_then(|id| workspace.agent_run(id))
        .or_else(|| workspace.data.agent_runs.last());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("Agent: {}   ", app.agent.profile.name), theme.agent),
            Span::raw(if app.agent.available {
                "available"
            } else {
                "not on PATH"
            }),
        ]),
        Line::from(""),
    ];
    if let Some(run) = run {
        let status = if run.status.is_active() {
            app.job_records
                .get(&run.job_id.unwrap_or(run.id))
                .and_then(|job| job.progress.clone())
                .unwrap_or_else(|| "Thinking...".into())
        } else {
            run.status.as_str().to_string()
        };
        let elapsed = run
            .job_id
            .and_then(|id| app.job_records.get(&id))
            .map(|job| job.elapsed_seconds(now()))
            .unwrap_or(0);
        lines.push(Line::from(format!(
            "{}  Status: {status}   Elapsed: {:02}:{:02}",
            run.short_ref(),
            elapsed / 60,
            elapsed % 60
        )));
        lines.push(Line::from(Span::styled(
            format!("Q  {}", sanitize_line(&run.question)),
            theme.header,
        )));
        lines.push(Line::from(""));
        let body = app
            .streaming
            .get(&run.job_id.unwrap_or(run.id))
            .cloned()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| run.response.clone());
        for line in sanitize_text(&body).lines() {
            lines.push(Line::from(line.to_string()));
        }
        if let Some(error) = &run.error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                sanitize_line(error).into_owned(),
                theme.error,
            )));
        }
    } else {
        lines.push(Line::from(
            "Press a to ask a question about the current page and evidence.",
        ));
        if !app.agent.available {
            for line in app.agent.message.as_deref().unwrap_or_default().lines() {
                lines.push(Line::from(Span::styled(line.to_string(), theme.muted)));
            }
        }
    }
    if workspace.data.agent_runs.len() > 1 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("History", theme.header)));
        for previous in workspace.data.agent_runs.iter().rev().take(8) {
            let marker = if Some(previous.id) == app.agent_run_id {
                ">"
            } else {
                " "
            };
            lines.push(Line::from(format!(
                "{marker} {}  {}  {}",
                previous.short_ref(),
                previous.status.symbol(theme.glyphs.ascii),
                ellipsize(&previous.question, 50, theme.glyphs.ascii)
            )));
        }
    }
    render_board_lines(frame, area, "Agent", lines, focused, theme, app, app.scroll);
}

fn jobs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let columns = [
        Column {
            key: SortKey::Name,
            header: "JOB",
            min: 8,
            weight: 1,
            align: Align::Left,
        },
        Column {
            key: SortKey::Status,
            header: "STATUS",
            min: 8,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Seen,
            header: "TIME",
            min: 6,
            weight: 0,
            align: Align::Right,
        },
    ];
    let mut rows = Vec::new();
    for job_id in app.listed_job_ids() {
        let stored = app
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.job(job_id));
        let live = app.job_records.get(&job_id);
        let Some(label) = live
            .map(|job| job.label.as_str())
            .or_else(|| stored.map(|job| job.label.as_str()))
        else {
            continue;
        };
        let status = live
            .map(|job| job.status)
            .or_else(|| stored.map(|job| job.status))
            .unwrap_or(crate::model::JobStatus::Queued);
        let glyph = match status {
            crate::model::JobStatus::Running | crate::model::JobStatus::Queued => {
                if live.is_some_and(|job| job.kind == "agent") {
                    app.motion.agent_glyph(&theme.glyphs)
                } else {
                    app.motion.job_spinner_glyph(&theme.glyphs)
                }
            }
            crate::model::JobStatus::Succeeded => theme.glyphs.done,
            crate::model::JobStatus::Failed => theme.glyphs.failed,
            crate::model::JobStatus::Cancelled => theme.glyphs.refused,
        };
        let elapsed = live
            .map(|job| job.elapsed_seconds(now()))
            .or_else(|| stored.map(|job| job.elapsed_seconds(now())))
            .unwrap_or(0);
        rows.push(vec![
            TableCell::text(label.to_string()),
            TableCell::text(format!("{glyph} {}", status.as_str())),
            TableCell::text(format!("{elapsed}s")),
        ]);
    }
    let empty = table_empty(
        app,
        theme,
        "No jobs yet",
        "Collect on a target to start a job.",
        "c collect",
    );
    let count = rows.len();
    let total = app.unfiltered_job_count();
    draw_table_board(
        frame, area, "JOBS", &columns, &rows, app, focused, theme, count, total, empty,
    );
}

fn artifacts(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items: Vec<String> = app
        .workspace
        .as_ref()
        .map(|workspace| {
            workspace
                .data
                .artifacts
                .iter()
                .map(|artifact| {
                    format!(
                        "{}  {:<14}  {}  {}",
                        artifact.short_ref(),
                        artifact.kind.label(),
                        crate::sanitize::format_bytes(artifact.size_bytes.max(0) as usize),
                        artifact.name
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    render_board_list(
        frame,
        area,
        "Artifacts",
        items,
        app.selected,
        focused,
        theme,
        app,
    );
}

fn tools(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let columns = [
        Column {
            key: SortKey::Status,
            header: "ST",
            min: 3,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Name,
            header: "NAME",
            min: 8,
            weight: 1,
            align: Align::Left,
        },
        Column {
            key: SortKey::Kind,
            header: "ACTIVITY",
            min: 8,
            weight: 0,
            align: Align::Left,
        },
        Column {
            key: SortKey::Value,
            header: "STATUS",
            min: 10,
            weight: 1,
            align: Align::Left,
        },
    ];
    let mut rows = Vec::new();
    for index in app.listed_tool_indices() {
        let Some(entry) = app.tools.tools().get(index) else {
            continue;
        };
        let refused = entry.definition.activity == Activity::Active
            || entry
                .problem
                .as_deref()
                .is_some_and(|problem| problem.contains("refused"));
        let mark = if refused {
            theme.glyphs.refused
        } else if entry.is_available() {
            theme.glyphs.done
        } else {
            theme.glyphs.missing()
        };
        let status = if refused {
            format!(
                "refused: {}",
                entry
                    .problem
                    .as_deref()
                    .unwrap_or("os-int observes public information only")
            )
        } else if entry.is_available() {
            "passive".into()
        } else {
            "missing".into()
        };
        let mark_style = if refused {
            theme.err
        } else if entry.is_available() {
            theme.ok
        } else {
            theme.muted
        };
        rows.push(vec![
            TableCell::styled(mark.to_string(), mark_style),
            TableCell::text(entry.definition.name.clone()),
            TableCell::text(entry.definition.activity.as_str().to_string()),
            TableCell::styled(status, if refused { theme.err } else { theme.fg }),
        ]);
    }
    let empty = table_empty(
        app,
        theme,
        "No tools configured",
        "Passive built-ins appear here. Active tools are refused.",
        "Ctrl+K doctor",
    );
    let count = rows.len();
    draw_table_board(
        frame,
        area,
        "TOOLS",
        &columns,
        &rows,
        app,
        focused,
        theme,
        count,
        app.tools.tools().len(),
        empty,
    );
}

fn settings(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let lines = vec![
        Line::from(Span::styled("Agent", theme.header)),
        Line::from(format!("  profile     {}", app.config.agent.active)),
        Line::from(format!("  program     {}", app.agent.profile.program)),
        Line::from(format!(
            "  timeout     {}s",
            app.config.agent.timeout_seconds
        )),
        Line::from(format!(
            "  available   {}",
            if app.agent.available { "yes" } else { "no" }
        )),
        Line::from(""),
        Line::from(Span::styled("Interface", theme.header)),
        Line::from(format!("  theme       {}", app.config.ui.theme)),
        Line::from(format!("  color       {}", app.config.ui.color)),
        Line::from(format!(
            "  ascii       {}",
            if app.config.ui.ascii { "yes" } else { "no" }
        )),
        Line::from(format!(
            "  editor      {}",
            app.config
                .ui
                .editor
                .as_deref()
                .unwrap_or("$VISUAL / $EDITOR / vi")
        )),
        Line::from(""),
        Line::from(Span::styled("Network", theme.header)),
        Line::from(format!(
            "  timeout     {}s",
            app.config.network.request_timeout_seconds
        )),
        Line::from(format!(
            "  proxy       {}",
            app.config
                .network
                .proxy
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(theme.glyphs.dash())
        )),
        Line::from(""),
        Line::from(Span::styled("Storage", theme.header)),
        Line::from(format!("  config      {}", app.paths.config_file.display())),
        Line::from(format!("  data        {}", app.paths.data_dir.display())),
        Line::from(format!("  database    {}", app.paths.database().display())),
        Line::from(""),
        Line::from(Span::styled(
            "Press e to edit the active agent profile.",
            theme.muted,
        )),
    ];
    render_board_lines(frame, area, "Settings", lines, focused, theme, app, 0);
}

fn draw_inspector(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focused = app.pane == Pane::Detail;
    let Some(workspace) = app.workspace.as_ref() else {
        frame.render_widget(
            Paragraph::new(pane_title("Inspector", focused, theme)),
            area,
        );
        return;
    };
    let mut lines = vec![inspector_heading(app, workspace, theme, focused)];
    if app.view == View::Review {
        lines.extend(review_inspector_lines(
            app,
            workspace,
            theme,
            area.height.saturating_sub(1) as usize,
        ));
    } else if app.view == View::Overview && app.inspector_tab == InspectorTab::Props {
        lines.extend(inspector_tab_lines(app, workspace, theme));
    } else {
        lines.extend(tab_bar(app, theme));
        lines.extend(inspector_tab_lines(app, workspace, theme));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll / 2, 0)),
        area,
    );
}

fn inspector_heading(
    app: &App,
    workspace: &Workspace,
    theme: &Theme,
    focused: bool,
) -> Line<'static> {
    let name = if app.view == View::Review {
        if let Some(item) = app.selected_review_item() {
            if item.written_as.is_some() {
                "Will write".to_string()
            } else {
                item.group.clone()
            }
        } else if let Some(run) = app.agent_run_id.and_then(|id| workspace.agent_run(id)) {
            run.short_ref().to_string()
        } else {
            "Will write".to_string()
        }
    } else if app.view == View::Evidence {
        match app.listed_evidence().get(app.selected) {
            Some(EvidenceRow::Finding(id)) => workspace
                .finding(*id)
                .map(|finding| finding.title.clone())
                .unwrap_or_else(|| "Nothing selected".into()),
            Some(EvidenceRow::Source(id)) => workspace
                .source(*id)
                .map(|source| source.title.clone())
                .unwrap_or_else(|| "Nothing selected".into()),
            None => "Nothing selected".into(),
        }
    } else if app.view == View::Entities {
        inspector_entity(app, workspace)
            .map(|entity| entity.display_name().to_string())
            .unwrap_or_else(|| "Nothing selected".into())
    } else if let Some(entity) = inspector_entity(app, workspace) {
        entity.display_name().to_string()
    } else if let Some(page) = app.page_id.and_then(|id| workspace.page(id)) {
        page.title.clone()
    } else {
        workspace.investigation().name.clone()
    };
    let pulse = if focused {
        app.motion.frame_kind(super::motion::AnimKind::Focus)
    } else {
        None
    };
    super::widgets::pane_title_pulse(
        &format!("Inspector{}{name}", theme.glyphs.sep()),
        focused,
        theme,
        pulse,
    )
}

fn tab_bar(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let tabs = InspectorTab::ALL
        .iter()
        .map(|tab| {
            if *tab == app.inspector_tab {
                if theme.glyphs.ascii {
                    format!("[{}]", tab.label())
                } else {
                    format!("·{}·", tab.label())
                }
            } else {
                tab.label().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("  ");
    vec![Line::from(Span::styled(
        format!("TABS  {tabs}"),
        theme.muted,
    ))]
}

fn inspector_tab_lines(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let board_empty = match app.view {
        View::Entities if !app.time_lens => app.listed_entities().is_empty(),
        View::Evidence if !app.time_lens => app.listed_evidence().is_empty(),
        _ => false,
    };
    if board_empty {
        return vec![Line::from(Span::styled("Nothing selected.", theme.muted))];
    }
    if app.view == View::Overview && app.inspector_tab == InspectorTab::Props {
        return case_inspector_props(app, workspace, theme);
    }
    match app.inspector_tab {
        InspectorTab::Props => {
            let mut lines = detail_lines(app, workspace, theme);
            if lines.is_empty() {
                lines.push(Line::from(Span::styled("Nothing selected.", theme.muted)));
            }
            lines
        }
        InspectorTab::Sources => inspector_sources(app, workspace, theme),
        InspectorTab::Notes => inspector_notes(app, workspace, theme),
        InspectorTab::History => inspector_history(app, workspace, theme),
    }
}

fn case_inspector_props(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let source_ids = inspector_source_ids(app, workspace);
    if let Some(entity) = inspector_entity(app, workspace) {
        let facts = entity.fields.len()
            + workspace.outgoing(entity.id).len()
            + workspace.incoming(entity.id).len();
        lines.push(Line::from(format!(
            "{}  {}",
            Glyphs::entity_tag(&entity.kind),
            entity.display_name()
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} facts{}{} sources",
                theme.glyphs.provenance(entity.provenance),
                facts.max(entity.source_ids.len()),
                theme.glyphs.sep(),
                source_ids.len().max(entity.source_ids.len())
            ),
            theme.muted,
        )));
    } else if let Some(page) = app.page_id.and_then(|id| workspace.page(id)) {
        lines.push(Line::from(format!(
            "{}  {}",
            page.kind.label(),
            page.target.as_deref().unwrap_or(page.title.as_str())
        )));
        lines.push(Line::from(Span::styled(
            format!("0 facts{}{} sources", theme.glyphs.sep(), source_ids.len()),
            theme.muted,
        )));
    } else {
        lines.push(Line::from(Span::styled("Nothing selected.", theme.muted)));
        return lines;
    }
    lines.extend(case_source_chain(app, workspace, theme));
    lines.extend(inspector_notes(app, workspace, theme));
    lines
}

fn case_source_chain(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("SOURCES", theme.header))];
    let ids = inspector_source_ids(app, workspace);
    if ids.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No sources on this object.",
            theme.muted,
        )));
        return lines;
    }
    for (index, id) in ids.iter().take(4).enumerate() {
        if let Some(source) = workspace.source(*id) {
            let collector = source.collector.as_deref().unwrap_or(source.kind.as_str());
            lines.push(Line::from(vec![
                Span::styled(format!("  S{} ", index + 1), theme.muted),
                provenance_glyph(source.provenance, theme),
                Span::raw(format!(
                    "  {collector}  {}",
                    relative_time(source.retrieved_at)
                )),
            ]));
        }
    }
    lines
}

fn inspector_sources(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("SOURCES", theme.header))];
    let ids = inspector_source_ids(app, workspace);
    if ids.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No sources on this object.",
            theme.muted,
        )));
        return lines;
    }
    for (index, id) in ids.iter().enumerate() {
        if let Some(source) = workspace.source(*id) {
            let when = relative_time(source.retrieved_at);
            let collector = source.collector.as_deref().unwrap_or(source.kind.as_str());
            lines.push(Line::from(vec![
                Span::styled(format!("  S{} ", index + 1), theme.muted),
                provenance_glyph(source.provenance, theme),
                Span::raw(format!("  {collector}  {when}")),
            ]));
            if !source.title.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("     {}", ellipsize(&source.title, 36, theme.glyphs.ascii)),
                    theme.muted,
                )));
            }
        }
    }
    lines
}

fn inspector_source_ids(app: &App, workspace: &Workspace) -> Vec<uuid::Uuid> {
    if app.view == View::Evidence {
        match app.listed_evidence().get(app.selected) {
            Some(EvidenceRow::Finding(id)) => {
                return workspace
                    .finding(*id)
                    .map(|finding| finding.source_ids.clone())
                    .unwrap_or_default();
            }
            Some(EvidenceRow::Source(id)) => return vec![*id],
            None => {}
        }
    }
    if let Some(entity) = app
        .entity_id
        .and_then(|id| workspace.entity(workspace.canonical_id(id)))
    {
        let mut ids = entity.source_ids.clone();
        if app.view == View::Overview
            && let Some(page_id) = app.page_id
        {
            for source in &workspace.data.sources {
                if source.page_id == Some(page_id) && !ids.contains(&source.id) {
                    ids.push(source.id);
                }
            }
        }
        return ids;
    }
    if let Some(finding) = app.finding_id.and_then(|id| workspace.finding(id)) {
        return finding.source_ids.clone();
    }
    if app.view == View::Overview
        && let Some(page_id) = app.page_id
    {
        return workspace
            .data
            .sources
            .iter()
            .filter(|source| source.page_id == Some(page_id))
            .map(|source| source.id)
            .collect();
    }
    Vec::new()
}

fn entity_inspector_props(
    workspace: &Workspace,
    entity: &Entity,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let provenances = entity_provenances(entity, workspace);
    let facts = entity.fields.len()
        + workspace.outgoing(entity.id).len()
        + workspace.incoming(entity.id).len();
    let mut heading = vec![Span::raw(format!(
        "{}  {}",
        Glyphs::entity_tag(&entity.kind),
        entity.display_name()
    ))];
    heading.push(Span::raw("  "));
    for provenance in &provenances {
        heading.push(provenance_glyph(*provenance, theme));
    }
    let mut lines = vec![
        Line::from(heading),
        Line::from(Span::styled(
            format!(
                "{} {} facts{}{} sources",
                theme.glyphs.provenance(entity.provenance),
                facts.max(entity.source_ids.len()),
                theme.glyphs.sep(),
                entity.source_ids.len()
            ),
            theme.muted,
        )),
    ];
    if !entity.fields.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("PROPERTIES", theme.header)));
        for (key, value) in entity.fields.iter().take(12) {
            let source_ref = field_footnote(entity, workspace, key, value);
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "  {key:<12}  {}  ",
                    ellipsize(value, 18, theme.glyphs.ascii)
                )),
                provenance_glyph(entity.provenance, theme),
                Span::styled(source_ref, theme.muted),
            ]));
        }
    }
    let outgoing = workspace.outgoing(entity.id);
    let incoming = workspace.incoming(entity.id);
    let link_count = outgoing.len() + incoming.len();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("LINKS  ({link_count})"),
        theme.header,
    )));
    for rel in outgoing {
        if let Some(to) = workspace.entity(rel.to_id) {
            lines.push(Line::from(format!(
                "  {} {}  {} {}",
                theme.glyphs.edge_out,
                rel.kind,
                Glyphs::entity_tag(&to.kind),
                to.display_name()
            )));
        }
    }
    for rel in incoming {
        if let Some(from) = workspace.entity(rel.from_id) {
            lines.push(Line::from(format!(
                "  {} {}  {} {}",
                theme.glyphs.edge_in,
                rel.kind,
                Glyphs::entity_tag(&from.kind),
                from.display_name()
            )));
        }
    }
    if link_count == 0 {
        lines.push(Line::from(Span::styled("  No links yet.", theme.muted)));
    }
    lines
}

fn inspector_notes(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("NOTES  e edit", theme.header))];
    if let Some(entity) = inspector_entity(app, workspace)
        && !entity.notes.trim().is_empty()
    {
        for line in entity.notes.lines().take(6) {
            lines.push(Line::from(sanitize_line(line).into_owned()));
        }
    }
    if let Some(page) = app.page_id.and_then(|id| workspace.page(id))
        && !page.notes.trim().is_empty()
    {
        for line in page.notes.lines().take(8) {
            lines.push(Line::from(sanitize_line(line).into_owned()));
        }
    }
    for note in workspace.data.notes.iter().take(6) {
        lines.push(Line::from(format!(
            "  {}  {}",
            note.short_ref(),
            ellipsize(note.headline(), 36, theme.glyphs.ascii)
        )));
    }
    if lines.len() == 1 {
        lines.push(Line::from(Span::styled(
            "  No notes yet. e edits in $EDITOR.",
            theme.muted,
        )));
    }
    lines
}

fn inspector_entity<'a>(app: &App, workspace: &'a Workspace) -> Option<&'a Entity> {
    app.selected_entity_id()
        .and_then(|id| workspace.entity(workspace.canonical_id(id)))
}

fn inspector_primary_id(app: &App, workspace: &Workspace) -> Option<uuid::Uuid> {
    if app.view == View::Evidence {
        return match app.listed_evidence().get(app.selected) {
            Some(EvidenceRow::Finding(id)) => Some(*id),
            Some(EvidenceRow::Source(id)) => Some(*id),
            None => None,
        };
    }
    inspector_entity(app, workspace)
        .map(|entity| entity.id)
        .or(app.finding_id)
        .or(app.source_id)
        .or(app.page_id)
}

fn inspector_history(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("HISTORY", theme.header))];
    let Some(id) = inspector_primary_id(app, workspace) else {
        lines.push(Line::from(Span::styled(
            "  No history for this object.",
            theme.muted,
        )));
        return lines;
    };
    let entries: Vec<_> = workspace
        .data
        .audit
        .iter()
        .rev()
        .filter(|entry| entry.target_ids.contains(&id))
        .take(8)
        .collect();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No history for this object.",
            theme.muted,
        )));
        return lines;
    }
    for entry in entries {
        lines.push(Line::from(format!(
            "  {}  {}",
            entry.short_ref(),
            ellipsize(&entry.summary, 36, theme.glyphs.ascii)
        )));
        lines.push(Line::from(Span::styled(
            format!("     {}", entry.occurred_at.format("%Y-%m-%d %H:%M UTC")),
            theme.muted,
        )));
    }
    lines
}

fn review_inspector_lines(
    app: &App,
    workspace: &Workspace,
    theme: &Theme,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let item = app.selected_review_item();
    let run_id = item
        .as_ref()
        .and_then(|item| item.run_id)
        .or(app.agent_run_id);
    let drawer = max_lines <= 12;
    let preview_cap = if drawer { 2 } else { 8 };
    let rationale_cap = if drawer { 1 } else { 6 };
    let mut lines = Vec::new();
    if let Some(item) = &item {
        for line in item.preview.iter().take(preview_cap) {
            lines.push(Line::from(line.clone()));
        }
        lines.push(Line::from(Span::styled("CITED EVIDENCE", theme.header)));
        if item.citations.is_empty() {
            lines.push(Line::from(Span::styled("  None cited.", theme.muted)));
        }
        for citation in &item.citations {
            let mark = if citation.present {
                theme.glyphs.done
            } else {
                theme.glyphs.failed
            };
            let style = if citation.present {
                theme.fg
            } else {
                theme.err
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {mark} {}  {}",
                    citation.reference,
                    ellipsize(&citation.label, 28, theme.glyphs.ascii)
                ),
                style,
            )));
        }
        if !item.rationale.is_empty() {
            lines.push(Line::from(Span::styled("RATIONALE", theme.ai)));
            for line in item.rationale.lines().take(rationale_cap) {
                lines.push(Line::from(format!("  {}", sanitize_line(line))));
            }
        }
    }
    if let Some(run) = run_id.and_then(|id| workspace.agent_run(id)) {
        lines.push(Line::from(Span::styled(
            format!("TRANSCRIPT  {}", run.short_ref()),
            theme.header,
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "Q  {}",
                ellipsize(&sanitize_line(&run.question), 36, theme.glyphs.ascii)
            ),
            theme.muted,
        )));
        let body = app
            .streaming
            .get(&run.job_id.unwrap_or(run.id))
            .cloned()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| run.response.clone());
        let remaining = max_lines.saturating_sub(lines.len().saturating_add(1));
        let body_text = sanitize_text(&body);
        let transcript: Vec<&str> = body_text.lines().collect();
        let take = remaining.max(1).min(if drawer { 3 } else { 12 });
        for line in transcript.iter().take(take) {
            lines.push(Line::from((*line).to_string()));
        }
        if transcript.len() > take {
            lines.push(Line::from(Span::styled(
                format!("  +{} more in transcript", transcript.len() - take),
                theme.muted,
            )));
        }
    } else if item.is_none() {
        lines.push(Line::from(Span::styled("Nothing selected.", theme.muted)));
    }
    lines
}

fn detail_lines(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    match app.view {
        View::Audit => {
            if let Some(entry) = workspace.data.audit.get(app.selected) {
                let mut lines = vec![
                    Line::from(format!("{}  {}", entry.short_ref(), entry.kind.label())),
                    Line::from(format!("When  {}", entry.occurred_at.to_rfc3339())),
                    Line::from(format!("Actor  {}@{}", entry.actor, entry.machine)),
                    Line::from(entry.summary.clone()),
                ];
                append_fields(&mut lines, &entry.details, theme);
                return lines;
            }
        }
        View::Sources => {
            let source = app
                .source_id
                .and_then(|id| workspace.source(id))
                .or_else(|| workspace.data.sources.get(app.selected));
            if let Some(source) = source {
                let mut lines = vec![
                    Line::from(format!("{}  {}", source.short_ref(), source.title)),
                    Line::from(vec![
                        Span::raw(format!("{}  ", source.kind.label())),
                        provenance_span(source.provenance, theme),
                    ]),
                ];
                if let Some(url) = &source.url {
                    lines.push(Line::from(format!("URL  {url}")));
                }
                lines.push(Line::from(format!(
                    "Retrieved  {}",
                    source.retrieved_at.to_rfc3339()
                )));
                append_fields(&mut lines, &source.fields, theme);
                lines.push(Line::from(""));
                for line in sanitize_text(&source.content).lines().take(80) {
                    lines.push(Line::from(line.to_string()));
                }
                return lines;
            }
        }
        View::Entities | View::Graph => {
            if let Some(entity) = inspector_entity(app, workspace) {
                return entity_inspector_props(workspace, entity, theme);
            }
        }
        View::Evidence => match app.listed_evidence().get(app.selected).cloned() {
            Some(EvidenceRow::Finding(id)) => {
                if let Some(finding) = workspace.finding(id) {
                    return vec![
                        Line::from(format!("{}  {}", finding.short_ref(), finding.title)),
                        Line::from(vec![
                            provenance_span(finding.provenance, theme),
                            Span::raw(format!(
                                "  {}  [{}]",
                                finding.confidence.as_str(),
                                workspace.format_source_refs(&finding.source_ids)
                            )),
                        ]),
                        Line::from(""),
                        Line::from(sanitize_text(&finding.details).into_owned()),
                    ];
                }
            }
            Some(EvidenceRow::Source(id)) => {
                if let Some(source) = workspace.source(id) {
                    return vec![
                        Line::from(format!("{}  {}", source.short_ref(), source.title)),
                        Line::from(vec![
                            Span::raw(format!("{}  ", source.kind.label())),
                            provenance_span(source.provenance, theme),
                        ]),
                        Line::from(
                            sanitize_text(&source.content)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .to_string(),
                        ),
                    ];
                }
            }
            None => {}
        },
        View::Findings => {
            let finding = app
                .finding_id
                .and_then(|id| workspace.finding(id))
                .or_else(|| workspace.data.findings.get(app.selected));
            if let Some(finding) = finding {
                return vec![
                    Line::from(format!("{}  {}", finding.short_ref(), finding.title)),
                    Line::from(vec![
                        provenance_span(finding.provenance, theme),
                        Span::raw(format!(
                            "  {}  [{}]",
                            finding.confidence.as_str(),
                            workspace.format_source_refs(&finding.source_ids)
                        )),
                    ]),
                    Line::from(""),
                    Line::from(sanitize_text(&finding.details).into_owned()),
                ];
            }
        }
        View::Jobs => {
            if let Some(job_id) = app.listed_job_ids().get(app.selected).copied() {
                let stored = workspace.job(job_id);
                let live = app.job_records.get(&job_id);
                let description = live
                    .map(|job| job.description.clone())
                    .or_else(|| stored.map(|job| job.description.clone()))
                    .unwrap_or_default();
                let status = live
                    .map(|job| job.status)
                    .or_else(|| stored.map(|job| job.status))
                    .unwrap_or(crate::model::JobStatus::Queued);
                let program = live
                    .and_then(|job| job.program.clone())
                    .or_else(|| stored.and_then(|job| job.program.clone()));
                let args = live
                    .map(|job| job.args.clone())
                    .or_else(|| stored.map(|job| job.args.clone()))
                    .unwrap_or_default();
                let log = live
                    .map(|job| job.log.clone())
                    .or_else(|| stored.map(|job| job.log.clone()))
                    .unwrap_or_default();
                let stderr = live
                    .map(|job| job.stderr.clone())
                    .or_else(|| stored.map(|job| job.stderr.clone()))
                    .unwrap_or_default();
                let mut lines = vec![
                    Line::from(description),
                    Line::from(format!("Status   {status}")),
                    Line::from(format!("Program  {}", program.as_deref().unwrap_or("-"))),
                    Line::from(format!("Args     {}", args.join(" "))),
                ];
                if status.is_active() && !live.is_some_and(|job| job.kind == "agent") {
                    lines.extend(observing_radar_lines(app, theme));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(sanitize_text(&log).into_owned()));
                lines.push(Line::from(sanitize_text(&stderr).into_owned()));
                return lines;
            }
        }
        View::Artifacts => {
            if let Some(artifact) = workspace.data.artifacts.get(app.selected) {
                return vec![
                    Line::from(format!("{}  {}", artifact.short_ref(), artifact.name)),
                    Line::from(artifact.kind.label().to_string()),
                    Line::from(artifact.path.clone()),
                    Line::from(format!("{} bytes", artifact.size_bytes)),
                    Line::from(artifact.sha256.clone().unwrap_or_else(|| "-".into())),
                    Line::from(""),
                    Line::from("Enter to preview."),
                ];
            }
        }
        View::Timeline => {
            if app.timeline.nearby {
                if let Some(entity) = app.listed_nearby().into_iter().nth(app.selected) {
                    let mut lines = vec![
                        Line::from(format!("{}  {}", entity.short_ref(), entity.display_name())),
                        Line::from(format!(
                            "Nearby of {}",
                            app.timeline
                                .nearby_seed_id
                                .and_then(|id| workspace
                                    .entity(id)
                                    .map(|seed| seed.display_name().to_string()))
                                .unwrap_or_else(|| "-".into())
                        )),
                    ];
                    append_fields(&mut lines, &entity.fields, theme);
                    return lines;
                }
            } else if let Some(row) = app.selected_timeline_row() {
                let mut lines = vec![
                    Line::from(format!("{}  {}", row.short_ref, row.title)),
                    Line::from(format!(
                        "{}  {}  {}",
                        row.kind.label(),
                        row.started_at.to_rfc3339(),
                        if row.ended_at.is_some() {
                            "interval"
                        } else {
                            "instant"
                        }
                    )),
                ];
                if let Some(ended) = row.ended_at {
                    lines.push(Line::from(format!("Ended  {}", ended.to_rfc3339())));
                }
                lines.push(Line::from(format!("Origin  {:?}", row.origin)));
                if !row.entity_ids.is_empty() {
                    let names: Vec<String> = row
                        .entity_ids
                        .iter()
                        .filter_map(|id| {
                            workspace
                                .entity(*id)
                                .map(|entity| entity.display_name().to_string())
                        })
                        .collect();
                    lines.push(Line::from(format!("Entities  {}", names.join(", "))));
                }
                if !row.description.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(sanitize_text(&row.description).into_owned()));
                }
                return lines;
            }
        }
        View::Pages => {
            if let Some(page) = workspace.data.pages.get(app.selected) {
                return vec![
                    Line::from(format!("{}  {}", page.short_ref(), page.title)),
                    Line::from(format!(
                        "{}  {}",
                        page.kind.label(),
                        page.target.as_deref().unwrap_or("-")
                    )),
                    Line::from(""),
                    Line::from(sanitize_text(&page.notes).into_owned()),
                ];
            }
        }
        _ => {
            if let Some(page) = app.page_id.and_then(|id| workspace.page(id)) {
                return vec![
                    Line::from(page.title.clone()),
                    Line::from(page.kind.label().to_string()),
                    Line::from(page.target.clone().unwrap_or_default()),
                ];
            }
        }
    }
    vec![Line::from(Span::styled("Nothing selected.", theme.muted))]
}

fn field_footnote(entity: &Entity, workspace: &Workspace, key: &str, value: &str) -> String {
    let chain = &entity.source_ids;
    let from_source = chain.iter().position(|id| {
        workspace
            .source(*id)
            .is_some_and(|source| source_supplied_field(source, key, value))
    });
    let from_member = from_source.or_else(|| {
        workspace
            .constituents_of(entity.id)
            .into_iter()
            .find_map(|member| {
                if member.fields.get(key) != Some(value) {
                    return None;
                }
                member
                    .source_ids
                    .iter()
                    .find_map(|id| chain.iter().position(|candidate| candidate == id))
            })
    });
    from_member
        .map(|index| format!(" S{}", index + 1))
        .unwrap_or_default()
}

fn source_supplied_field(source: &crate::model::Source, key: &str, value: &str) -> bool {
    source
        .fields
        .iter()
        .any(|(field, found)| field.eq_ignore_ascii_case(key) && found == value)
        || (!value.is_empty() && source.content.contains(value))
}

fn append_fields(lines: &mut Vec<Line<'static>>, fields: &crate::model::Fields, theme: &Theme) {
    if fields.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Fields", theme.header)));
    for (key, value) in fields.iter().take(16) {
        lines.push(Line::from(format!(
            "  {key}  {}",
            ellipsize(value, 60, theme.glyphs.ascii)
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::{field_footnote, source_supplied_field};
    use crate::model::{
        Entity, EntityKind, Fields, Investigation, InvestigationData, Provenance, Source,
        SourceKind, Workspace, now,
    };

    fn source_with_field(seq: i64, key: &str, value: &str) -> Source {
        let timestamp = now();
        let mut fields = Fields::new();
        fields.push(key, value);
        Source {
            id: uuid::Uuid::new_v4(),
            investigation_id: uuid::Uuid::nil(),
            seq,
            kind: SourceKind::Dns,
            title: format!("source {seq}"),
            url: None,
            retrieved_at: timestamp,
            last_seen_at: None,
            published_at: None,
            author: None,
            collector: Some("dns".into()),
            content_hash: None,
            snapshot_path: None,
            content: String::new(),
            fields,
            notes: String::new(),
            provenance: Provenance::Collected,
            page_id: None,
            job_id: None,
        }
    }

    #[test]
    fn props_footnote_cites_the_source_that_supplied_the_field() {
        let dns = source_with_field(1, "registrar", "examplereg");
        let intel = source_with_field(2, "asn", "AS1");
        let timestamp = now();
        let mut entity = Entity {
            id: uuid::Uuid::new_v4(),
            investigation_id: uuid::Uuid::nil(),
            seq: 1,
            kind: EntityKind::Domain,
            value: "example.com".into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Collected,
            fields: Fields::new(),
            source_ids: vec![dns.id, intel.id],
            created_at: timestamp,
            updated_at: timestamp,
        };
        entity.fields.push("registrar", "examplereg");
        entity.fields.push("asn", "AS1");
        let mut data = InvestigationData::empty(Investigation {
            id: uuid::Uuid::nil(),
            name: "case".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: timestamp,
            updated_at: timestamp,
        });
        data.entities.push(entity.clone());
        data.sources.extend([dns.clone(), intel.clone()]);
        let workspace = Workspace::new(data);
        assert!(source_supplied_field(&dns, "registrar", "examplereg"));
        assert_eq!(
            field_footnote(&entity, &workspace, "registrar", "examplereg"),
            " S1"
        );
        assert_eq!(field_footnote(&entity, &workspace, "asn", "AS1"), " S2");
        assert_eq!(
            field_footnote(&entity, &workspace, "role", "guess"),
            "",
            "unknown fields must not fall back to S1"
        );
    }
}
