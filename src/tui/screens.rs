//! Full-screen views: welcome, investigation workspace, and per-section panes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::filter::{SortKey, entity_provenances, entity_seen_at};
use crate::app::{App, EvidenceRow, InspectorTab, Overlay, Pane, View};
use crate::doctor::CheckStatus;
use crate::model::{
    Entity, FacetAxis, GraphFilters, LinkDirection, Workspace, layout_columns, now,
};
use crate::sanitize::{sanitize_line, sanitize_text};
use crate::tools::Activity;

use super::glyphs::Glyphs;
use super::table::{Align, Column, TableCell, provenance_cells, relative_time, render_table};
use super::theme::Theme;
use super::widgets::{
    empty_state, error_panel, input_line, list_from, loading_state, pane_block, pane_title,
    provenance_glyph, provenance_span, rule_span, truncate,
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
        Line::from("Create an investigation, collect public evidence, take notes,"),
        Line::from("and ask a local agent CLI (Claude Code, Codex, or your own)."),
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
            Span::styled(format!("  {}", truncate(&check.detail, 36)), theme.muted),
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
const INSPECTOR_BREAK: u16 = 110;

pub fn workspace(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.board_zoomed {
        draw_main(frame, area, app, theme);
        return;
    }
    if area.width < INSPECTOR_BREAK {
        let drawer = (area.height as u32 * 40 / 100).max(6) as u16;
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
        return;
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
}

fn draw_rail(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, digits_only: bool) {
    let focused = app.pane == Pane::Nav;
    let review = app.pending_review_count();
    let jobs = app
        .job_records
        .values()
        .filter(|job| job.status.is_active())
        .count();
    let mut lines = vec![pane_title("Rail", focused, theme)];
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
            text = format!("{text} {}", theme.glyphs.spinner[0]);
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
    let Some(workspace) = app.workspace.as_ref() else {
        return;
    };
    let page = app.page_id.and_then(|id| workspace.page(id));
    let mut lines = Vec::new();
    if let Some(page) = page {
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", page.short_ref()), theme.muted),
            Span::styled(page.title.clone(), theme.title),
            Span::raw(format!("  {}", page.kind.label())),
        ]));
        if let Some(target) = &page.target {
            lines.push(Line::from(format!("Target  {target}")));
        }
        lines.push(Line::from(""));
        if !page.notes.trim().is_empty() {
            lines.push(Line::from(Span::styled("Notes", theme.header)));
            for line in page.notes.lines().take(6) {
                lines.push(Line::from(sanitize_line(line).into_owned()));
            }
            lines.push(Line::from(""));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(workspace.investigation().name.clone(), theme.title),
            Span::raw("  "),
            Span::styled(
                workspace.investigation().marking.label().to_string(),
                theme.header,
            ),
        ]));
        lines.push(Line::from(
            sanitize_line(&workspace.investigation().description).into_owned(),
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Press n to add a Domain, IP, URL, or Repository page.",
        ));
    }
    lines.push(Line::from(Span::styled("Summary", theme.header)));
    lines.push(Line::from(format!(
        "  marking {}   role {}",
        workspace.investigation().marking.label(),
        app.current_role_label()
    )));
    lines.push(Line::from(format!(
        "  {} pages   {} sources   {} findings   {} entities   {} jobs   {} audit",
        workspace.data.pages.len(),
        workspace.data.sources.len(),
        workspace.data.findings.len(),
        workspace.data.entities.len(),
        workspace.data.jobs.len(),
        workspace.data.audit.len()
    )));
    if let Some(page) = page {
        let entities: Vec<_> = workspace
            .page_entity_ids(page.id)
            .into_iter()
            .filter_map(|id| workspace.entity(id))
            .collect();
        if !entities.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Entities on this page",
                theme.header,
            )));
            for entity in entities.into_iter().take(12) {
                lines.push(Line::from(vec![
                    Span::raw(format!(
                        "  {}  {:<10}  ",
                        entity.short_ref(),
                        entity.kind.label()
                    )),
                    Span::raw(truncate(entity.display_name(), 40)),
                    Span::raw("  "),
                    provenance_span(entity.provenance, theme),
                ]));
                if let Some(posture) = entity.fields.get("diagnostics") {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", truncate(posture, 72)),
                        theme.muted,
                    )));
                }
            }
        }
        let findings: Vec<_> = workspace
            .data
            .findings
            .iter()
            .filter(|finding| finding.page_id == Some(page.id))
            .collect();
        if !findings.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Findings", theme.header)));
            for finding in findings.into_iter().take(6) {
                lines.push(finding_line(workspace, finding, theme));
            }
        }
    }
    if app.has_active_jobs() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Running", theme.warning)));
        for job in app
            .job_records
            .values()
            .filter(|job| job.status.is_active())
        {
            lines.push(Line::from(format!(
                "  {} {}  {}",
                job.status.symbol(),
                job.label,
                job.progress.as_deref().unwrap_or(job.description.as_str())
            )));
        }
    }
    lines.push(Line::from(""));
    lines.insert(0, pane_title("TARGETS", focused, theme));
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
        "c collect · n new target",
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
                "n new · c collect",
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
            filter_line.push(Span::styled(format!("{total} → {shown}"), theme.muted));
            lines.push(Line::from(filter_line));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("/{filter}"), theme.accent),
                Span::raw("  "),
                Span::styled(format!("{total} → {shown}"), theme.muted),
            ]));
        }
    }
    if let Some(error) = error {
        lines.extend(error);
    }
    frame.render_widget(Paragraph::new(lines), split[0]);
    if rows.is_empty() {
        if let Some((job_ref, label)) = loading_job(app) {
            frame.render_widget(
                Paragraph::new(loading_state(&job_ref, &label, theme)),
                split[1],
            );
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
                truncate(&property.value, 56)
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
            truncate(&source.title, 48)
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
            truncate(&finding.title, 48)
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
                truncate(&constituent.label, 40)
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
    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(8),
        Constraint::Length(area.height.saturating_div(3).max(6)),
    ])
    .split(area);
    let visible = app.visible_graph();
    let chips = app.graph_facet_chips();
    let mut facet_lines = vec![Line::from(Span::styled(graph_title(app), theme.header))];
    facet_lines.push(graph_facet_line(app, &chips, theme));
    facet_lines.push(Line::from(Span::styled(
        graph_filter_line(&app.graph.filters),
        theme.muted,
    )));
    facet_lines.insert(0, pane_title("Facets", focused, theme));
    frame.render_widget(
        Paragraph::new(facet_lines).wrap(Wrap { trim: false }),
        rows[0],
    );
    let mut canvas = vec![pane_title("Neighborhood", focused, theme)];
    canvas.extend(graph_canvas_lines(app, &visible, theme));
    frame.render_widget(Paragraph::new(canvas).wrap(Wrap { trim: false }), rows[1]);
    let items: Vec<String> = visible
        .nodes
        .iter()
        .map(|node| graph_node_row(app, node.id, node.hop))
        .collect();
    let selected = if visible.nodes.is_empty() {
        0
    } else {
        app.graph
            .selected
            .min(visible.nodes.len().saturating_sub(1))
    };
    render_board_list(
        frame, rows[2], "Nodes", items, selected, focused, theme, app,
    );
}

fn graph_title(app: &App) -> String {
    let result = &app.graph.result;
    let cap = if result.truncated { " · capped" } else { "" };
    format!(
        "{} hop(s) · {} nodes · {} edges · cap {}/{}{cap}",
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
            spans.push(Span::styled(" │ ", theme.muted));
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

fn graph_filter_line(filters: &GraphFilters) -> String {
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
    format!("Filter {}   x clear", parts.join(" · "))
}

fn graph_canvas_lines(
    app: &App,
    visible: &crate::model::VisibleGraph,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let _ = theme;
    if visible.nodes.is_empty() {
        return vec![Line::from(
            "No nodes. Collect on a domain page, then press g or s.".to_string(),
        )];
    }
    let Some(workspace) = app.workspace.as_ref() else {
        return vec![Line::from("No investigation.".to_string())];
    };
    let selected = visible.nodes.get(app.graph.selected).map(|node| node.id);
    let columns = layout_columns(&visible.nodes);
    let mut lines = Vec::new();
    let header: String = columns
        .iter()
        .map(|(hop, _)| format!("Hop {hop:<12}"))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(Line::from(header));
    let height = columns
        .iter()
        .map(|(_, ids)| ids.len())
        .max()
        .unwrap_or(0)
        .min(12);
    for row in 0..height {
        let mut cells = Vec::new();
        for (_, ids) in &columns {
            let cell = ids
                .get(row)
                .and_then(|id| workspace.entity(*id))
                .map(|entity| {
                    let mark = if selected == Some(entity.id) {
                        "*"
                    } else {
                        " "
                    };
                    format!(
                        "{mark}{:<14}",
                        truncate(
                            &format!("{} {}", entity.short_ref(), entity.display_name()),
                            14
                        )
                    )
                })
                .unwrap_or_else(|| " ".repeat(16));
            cells.push(cell);
        }
        lines.push(Line::from(cells.join(" ")));
    }
    let extra = columns
        .iter()
        .map(|(_, ids)| ids.len().saturating_sub(height))
        .sum::<usize>();
    if extra > 0 {
        lines.push(Line::from(format!("  +{extra} more in the node list")));
    }
    lines
}

fn graph_node_row(app: &App, id: uuid::Uuid, hop: u32) -> String {
    let Some(workspace) = app.workspace.as_ref() else {
        return format!("hop {hop}");
    };
    let Some(entity) = workspace.entity(id) else {
        return format!("hop {hop}  (missing entity)");
    };
    format!(
        "{}  {:<10}  hop {hop}  {}",
        entity.short_ref(),
        entity.kind.label(),
        entity.display_name()
    )
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
        "c collect · n new target",
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
            truncate(&entry.summary, 48)
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
            format!("Time lens {mode} · {filter} · x clear"),
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
                    .map(|ended| format!("→{}", ended.format("%H:%M")))
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
            truncate(&page.notes, 60)
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
                previous.status.symbol(),
                truncate(&previous.question, 50)
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
                theme.glyphs.spinner[0]
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
            "·"
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
                .unwrap_or("—")
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
        lines.extend(review_inspector_lines(app, workspace, theme));
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
    pane_title(&format!("Inspector · {name}"), focused, theme)
}

fn tab_bar(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let tabs = InspectorTab::ALL
        .iter()
        .map(|tab| {
            if *tab == app.inspector_tab {
                format!("·{}·", tab.label())
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
                    format!("     {}", truncate(&source.title, 36)),
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
        return entity.source_ids.clone();
    }
    if let Some(finding) = app.finding_id.and_then(|id| workspace.finding(id)) {
        return finding.source_ids.clone();
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
                "{} {} facts · {} sources",
                theme.glyphs.provenance(entity.provenance),
                facts.max(entity.source_ids.len()),
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
                Span::raw(format!("  {key:<12}  {}  ", truncate(value, 18))),
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
    let mut lines = vec![Line::from(Span::styled(
        "NOTES  e edit entity notes",
        theme.header,
    ))];
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
            truncate(note.headline(), 36)
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

fn inspector_object_ids(app: &App, workspace: &Workspace) -> Vec<uuid::Uuid> {
    let mut ids = Vec::new();
    if let Some(entity) = inspector_entity(app, workspace) {
        ids.push(entity.id);
        ids.extend(entity.source_ids.iter().copied());
    }
    if let Some(page_id) = app.page_id {
        ids.push(page_id);
    }
    if let Some(finding_id) = app.finding_id {
        ids.push(finding_id);
    }
    if let Some(source_id) = app.source_id {
        ids.push(source_id);
    }
    if app.view == View::Evidence {
        match app.listed_evidence().get(app.selected) {
            Some(EvidenceRow::Finding(id)) => ids.push(*id),
            Some(EvidenceRow::Source(id)) => ids.push(*id),
            None => {}
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn inspector_entity<'a>(app: &App, workspace: &'a Workspace) -> Option<&'a Entity> {
    app.selected_entity_id()
        .and_then(|id| workspace.entity(workspace.canonical_id(id)))
}

fn inspector_history(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("HISTORY", theme.header))];
    let ids = inspector_object_ids(app, workspace);
    let entries: Vec<_> = workspace
        .data
        .audit
        .iter()
        .rev()
        .filter(|entry| {
            if ids.is_empty() {
                return false;
            }
            entry.target_ids.iter().any(|id| ids.contains(id))
        })
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
            truncate(&entry.summary, 36)
        )));
        lines.push(Line::from(Span::styled(
            format!("     {}", entry.occurred_at.format("%Y-%m-%d %H:%M UTC")),
            theme.muted,
        )));
    }
    lines
}

fn review_inspector_lines(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    let item = app.selected_review_item();
    let run_id = item
        .as_ref()
        .and_then(|item| item.run_id)
        .or(app.agent_run_id);
    let mut lines = Vec::new();
    if let Some(item) = &item {
        for line in &item.preview {
            lines.push(Line::from(line.clone()));
        }
        lines.push(Line::from(""));
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
                format!("  {mark} {}  {}", citation.reference, citation.label),
                style,
            )));
        }
        if !item.rationale.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("RATIONALE ◆", theme.ai)));
            for line in item.rationale.lines().take(6) {
                lines.push(Line::from(format!("  ┊ {}", sanitize_line(line))));
            }
        }
    }
    if let Some(run) = run_id.and_then(|id| workspace.agent_run(id)) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("TRANSCRIPT  {}", run.short_ref()),
            theme.header,
        )));
        lines.push(Line::from(Span::styled(
            format!("Q  {}", sanitize_line(&run.question)),
            theme.muted,
        )));
        let body = app
            .streaming
            .get(&run.job_id.unwrap_or(run.id))
            .cloned()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| run.response.clone());
        for line in sanitize_text(&body).lines().take(12) {
            lines.push(Line::from(line.to_string()));
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
                return vec![
                    Line::from(description),
                    Line::from(format!("Status   {status}")),
                    Line::from(format!("Program  {}", program.as_deref().unwrap_or("-"))),
                    Line::from(format!("Args     {}", args.join(" "))),
                    Line::from(""),
                    Line::from(sanitize_text(&log).into_owned()),
                    Line::from(sanitize_text(&stderr).into_owned()),
                ];
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
        .or_else(|| chain.first().map(|_| 0))
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
        lines.push(Line::from(format!("  {key}  {}", truncate(value, 60))));
    }
}

fn finding_line(
    workspace: &Workspace,
    finding: &crate::model::Finding,
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {}  ", finding.short_ref())),
        provenance_span(finding.provenance, theme),
        Span::raw(format!(
            "  {}  [{}]",
            truncate(&finding.title, 42),
            workspace.format_source_refs(&finding.source_ids)
        )),
    ])
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
    }
}
