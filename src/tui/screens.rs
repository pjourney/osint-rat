//! Full-screen views: welcome, investigation workspace, and per-section panes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{App, Pane, View};
use crate::doctor::CheckStatus;
use crate::model::{FacetAxis, GraphFilters, LinkDirection, Workspace, layout_columns, now};
use crate::sanitize::{sanitize_line, sanitize_text};

use super::theme::Theme;
use super::widgets::{list_from, pane_block, provenance_span, truncate};

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
        welcome_item(0, app.selected, "[N] New Investigation", theme),
        welcome_item(1, app.selected, "[O] Open Investigation", theme),
        welcome_item(2, app.selected, "[D] Doctor", theme),
        welcome_item(3, app.selected, "[?] Help", theme),
        Line::from(""),
        Line::from(Span::styled("Recent investigations", theme.header)),
    ];
    let recent = app.investigations();
    if recent.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none yet — press n)",
            theme.muted,
        )));
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

pub fn workspace(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let narrow = area.width < 72;
    let hide_detail = area.width < 96;
    let nav_width = if narrow { 12 } else { 16 };
    let mut constraints = vec![Constraint::Length(nav_width), Constraint::Min(24)];
    if !hide_detail {
        constraints.push(Constraint::Length(area.width / 3));
    }
    let columns = Layout::horizontal(constraints).split(area);
    draw_nav(frame, columns[0], app, theme);
    draw_main(frame, columns[1], app, theme);
    if !hide_detail {
        draw_detail(frame, columns[2], app, theme);
    }
}

fn draw_nav(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let items: Vec<String> = app
        .nav_items()
        .into_iter()
        .map(|(label, _)| label.to_string())
        .collect();
    let list = list_from(items, app.nav_index, theme).block(pane_block(
        "Navigate",
        app.pane == Pane::Nav,
        theme,
    ));
    frame.render_widget(list, area);
}

fn draw_main(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focused = app.pane == Pane::Main;
    match app.view {
        View::Welcome => {}
        View::Overview => overview(frame, area, app, theme, focused),
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
        lines.push(Line::from(Span::styled(
            workspace.investigation().name.clone(),
            theme.title,
        )));
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
        "  {} pages   {} sources   {} findings   {} entities   {} jobs",
        workspace.data.pages.len(),
        workspace.data.sources.len(),
        workspace.data.findings.len(),
        workspace.data.entities.len(),
        workspace.data.jobs.len()
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
    lines.push(Line::from(Span::styled(
        "r run collectors   a ask agent   n new page   p commands",
        theme.muted,
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Overview", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

fn object_view(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let Some(view) = app.current_object_view() else {
        frame.render_widget(
            Paragraph::new("Select an entity or page, then press v or Enter.")
                .block(pane_block("Object", focused, theme)),
            area,
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
        lines.push(Line::from(Span::styled("  (none)", theme.muted)));
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
        lines.push(Line::from(Span::styled("  (none)", theme.muted)));
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
        lines.push(Line::from(Span::styled("  (none)", theme.muted)));
    }
    for finding in &view.findings {
        lines.push(Line::from(format!(
            "  {}  [{}]  {}",
            finding.reference,
            finding.provenance,
            truncate(&finding.title, 48)
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Object", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

fn graph(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
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
    frame.render_widget(
        Paragraph::new(facet_lines)
            .block(pane_block("Facets", focused, theme))
            .wrap(Wrap { trim: false }),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(graph_canvas_lines(app, &visible, theme))
            .block(pane_block("Neighborhood", focused, theme))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
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
    frame.render_widget(
        list_from(items, selected, theme).block(pane_block("Nodes", focused, theme)),
        rows[2],
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
        return "[ ]/] facet   f apply   x clear   +/- hops   s Search Around".into();
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
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Pages", focused, theme)),
        area,
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
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Sources", focused, theme)),
        area,
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
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Findings", focused, theme)),
        area,
    );
}

fn entities(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items = app
        .workspace
        .as_ref()
        .map(|workspace| {
            workspace
                .data
                .entities
                .iter()
                .map(|entity| {
                    format!(
                        "{}  {:<12}  {}  {}",
                        entity.short_ref(),
                        entity.kind.label(),
                        entity.provenance.badge(),
                        entity.display_name()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Entities", focused, theme)),
        area,
    );
}

fn timeline(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items = app
        .workspace
        .as_ref()
        .map(|workspace| {
            workspace
                .data
                .timeline
                .iter()
                .map(|event| {
                    format!(
                        "{}  {}  {:<16}  {}",
                        event.short_ref(),
                        event.occurred_at.format("%Y-%m-%d"),
                        event.kind.label(),
                        event.title
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Timeline", focused, theme)),
        area,
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
    if items.is_empty() {
        items.push("(no notes — press n)".into());
    }
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Notes", focused, theme)),
        area,
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
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Agent", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}

fn jobs(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let mut items: Vec<String> = Vec::new();
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
        let reference = stored
            .map(|job| job.short_ref().to_string())
            .or_else(|| live.map(|job| job.short_ref().to_string()))
            .unwrap_or_else(|| "-".into());
        let progress = live
            .and_then(|job| job.progress.clone())
            .or_else(|| stored.and_then(|job| job.progress.clone()));
        let description = live
            .map(|job| job.description.as_str())
            .or_else(|| stored.map(|job| job.description.as_str()))
            .unwrap_or("");
        items.push(format!(
            "{}  {}  {:<10}  {}  {}",
            reference,
            status.symbol(),
            label,
            progress.unwrap_or_default(),
            truncate(description, 40)
        ));
    }
    if items.is_empty() {
        items.push("(no jobs yet — press r on a Domain page)".into());
    }
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Jobs", focused, theme)),
        area,
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
    let items = if items.is_empty() {
        vec!["(no artifacts yet — reports, snapshots, and agent context packs appear here)".into()]
    } else {
        items
    };
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Artifacts", focused, theme)),
        area,
    );
}

fn tools(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let items: Vec<String> = app
        .tools
        .tools()
        .iter()
        .map(|entry| {
            let mark = if entry.is_available() { "✓" } else { "·" };
            format!(
                "{mark}  {:<12}  {:<8}  {}",
                entry.definition.name,
                entry.definition.activity.as_str(),
                entry.definition.description
            )
        })
        .collect();
    frame.render_widget(
        list_from(items, app.selected, theme).block(pane_block("Tools", focused, theme)),
        area,
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
                .unwrap_or("(none)")
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
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Settings", focused, theme)),
        area,
    );
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focused = app.pane == Pane::Detail;
    let Some(workspace) = app.workspace.as_ref() else {
        frame.render_widget(
            Paragraph::new("").block(pane_block("Detail", focused, theme)),
            area,
        );
        return;
    };
    let lines = detail_lines(app, workspace, theme);
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Detail", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll / 2, 0)),
        area,
    );
}

fn detail_lines(app: &App, workspace: &Workspace, theme: &Theme) -> Vec<Line<'static>> {
    match app.view {
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
            let entity = app
                .entity_id
                .and_then(|id| workspace.entity(id))
                .or_else(|| workspace.data.entities.get(app.selected));
            if let Some(entity) = entity {
                let mut lines = vec![
                    Line::from(format!("{}  {}", entity.short_ref(), entity.display_name())),
                    Line::from(vec![
                        Span::raw(format!("{}  ", entity.kind.label())),
                        provenance_span(entity.provenance, theme),
                    ]),
                    Line::from(format!(
                        "Sources  {}",
                        workspace.format_source_refs(&entity.source_ids)
                    )),
                ];
                append_fields(&mut lines, &entity.fields, theme);
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Relationships", theme.header)));
                for rel in workspace.outgoing(entity.id) {
                    if let Some(to) = workspace.entity(rel.to_id) {
                        lines.push(Line::from(format!(
                            "  --{}-->  {} {}",
                            rel.kind,
                            to.short_ref(),
                            to.display_name()
                        )));
                    }
                }
                for rel in workspace.incoming(entity.id) {
                    if let Some(from) = workspace.entity(rel.from_id) {
                        lines.push(Line::from(format!(
                            "  <--{}--  {} {}",
                            rel.kind,
                            from.short_ref(),
                            from.display_name()
                        )));
                    }
                }
                return lines;
            }
        }
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
