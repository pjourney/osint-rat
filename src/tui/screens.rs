//! Full-screen views: welcome, investigation workspace, and per-section panes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{App, Pane, View};
use crate::doctor::CheckStatus;
use crate::model::{Workspace, now};
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
    if let Some(workspace) = app.workspace.as_ref() {
        for job in &workspace.data.jobs {
            let live = app.job_records.get(&job.id);
            let status = live.map(|live| live.status).unwrap_or(job.status);
            let progress = live
                .and_then(|live| live.progress.clone())
                .or(job.progress.clone());
            items.push(format!(
                "{}  {}  {:<10}  {}  {}",
                job.short_ref(),
                status.symbol(),
                job.label,
                progress.unwrap_or_default(),
                truncate(&job.description, 40)
            ));
        }
    }
    for job in app.job_records.values() {
        if app
            .workspace
            .as_ref()
            .is_some_and(|workspace| workspace.job(job.id).is_some())
        {
            continue;
        }
        items.push(format!(
            "{}  {}  {}  {}",
            job.short_ref(),
            job.status.symbol(),
            job.label,
            job.description
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
                lines.push(Line::from(""));
                for line in sanitize_text(&source.content).lines().take(80) {
                    lines.push(Line::from(line.to_string()));
                }
                return lines;
            }
        }
        View::Entities => {
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
                    Line::from(""),
                    Line::from(Span::styled("Relationships", theme.header)),
                ];
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
            if let Some(job) = workspace.data.jobs.get(app.selected) {
                let live = app.job_records.get(&job.id);
                return vec![
                    Line::from(job.description.clone()),
                    Line::from(format!(
                        "Status   {}",
                        live.map(|live| live.status).unwrap_or(job.status)
                    )),
                    Line::from(format!(
                        "Program  {}",
                        job.program.as_deref().unwrap_or("-")
                    )),
                    Line::from(format!("Args     {}", job.args.join(" "))),
                    Line::from(""),
                    Line::from(sanitize_text(&job.log).into_owned()),
                    Line::from(sanitize_text(&job.stderr).into_owned()),
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
