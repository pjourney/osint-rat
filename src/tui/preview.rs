//! `os-int theme preview` — swatches, a sample cockpit strip, and colour-blind ΔE.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use color_eyre::eyre::{Result, eyre};

use crate::config::Config;

use super::glyphs::Glyphs;
use super::theme::{
    CLOSE_DELTA, ColorMode, CvdKind, DetectedDepth, Role, TerminalEnv, Theme, ThemeName,
    close_pairs, contrast_ratio, critical_delta, detect_color_depth, min_semantic_delta,
    resolve_theme_name,
};

/// Renders one theme, or all five, as plain text (no TTY required).
pub fn render_preview(
    config: &Config,
    env: &TerminalEnv,
    name: Option<&str>,
    all: bool,
) -> Result<String> {
    let color_mode = ColorMode::parse(&config.ui.color).unwrap_or(ColorMode::Auto);
    let detected = detect_color_depth(env, color_mode);
    let ascii = config.ui.ascii;
    let mut chunks = Vec::new();
    chunks.push(format!(
        "detected colour depth: {} ({})",
        detected.depth.as_str(),
        detected.reason
    ));
    chunks.push(format!("ui.ascii = {ascii}"));
    chunks.push(String::new());
    if all {
        for theme_name in ThemeName::ALL {
            chunks.push(render_one(theme_name, detected.clone(), ascii));
            chunks.push(String::new());
        }
    } else {
        let requested = match name {
            Some(value) => ThemeName::parse(value).ok_or_else(|| {
                eyre!(
                    "unknown theme {value:?}; valid themes: {}",
                    ThemeName::ALL
                        .iter()
                        .map(|theme| theme.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
            None => resolve_theme_name(&config.ui.theme, env, detected.depth),
        };
        chunks.push(render_one(requested, detected, ascii));
    }
    Ok(chunks.join("\n"))
}

fn render_one(requested: ThemeName, detected: DetectedDepth, ascii: bool) -> String {
    let theme = Theme::build(requested, detected, ascii);
    let mut lines = Vec::new();
    lines.push(format!("# {}", requested.label()));
    if theme.name != requested {
        lines.push(format!(
            "resolved: {} ({})",
            theme.name.label(),
            theme.degrade_hint.unwrap_or("forced Mono")
        ));
    } else {
        lines.push(format!("resolved: {}", theme.name.label()));
    }
    lines.push(format!(
        "depth: {} ({})",
        theme.depth.as_str(),
        theme.depth_reason
    ));
    if let Some(hint) = theme.degrade_hint {
        lines.push(format!("hint: {hint}"));
    }
    lines.push(String::new());
    lines.push(format!(
        "{:<8} {:<8} {:>4} {:<10} {:>7} {:>9}",
        "role", "hex", "256", "16", "vs bg", "vs panel"
    ));
    let bg = theme.spec(Role::Bg).rgb;
    let panel = theme.spec(Role::Panel).rgb;
    for role in Role::ALL {
        let spec = theme.spec(role);
        let hex = spec
            .rgb
            .map(|rgb| rgb.format_hex())
            .unwrap_or_else(|| theme.glyphs.dash().to_string());
        let index = if theme.name == ThemeName::Mono {
            theme.glyphs.dash().to_string()
        } else {
            spec.index256.to_string()
        };
        let vs_bg = contrast_cell(spec.rgb, bg, theme.glyphs.ascii);
        let vs_panel = contrast_cell(spec.rgb, panel, theme.glyphs.ascii);
        lines.push(format!(
            "{:<8} {:<8} {:>4} {:<10} {:>7} {:>9}",
            role.as_str(),
            hex,
            index,
            spec.ansi16_name,
            vs_bg,
            vs_panel
        ));
    }
    lines.push(String::new());
    lines.push("sample (80×12)".into());
    lines.push("-".repeat(80));
    for row in render_sample(&theme).lines() {
        lines.push(row.to_string());
    }
    lines.push("-".repeat(80));
    lines.push(String::new());
    lines.push("colour-blind ΔE (Machado 2009, severity 1.0)".into());
    if theme.name == ThemeName::Mono {
        lines.push("  Mono never relies on colour.".into());
    } else {
        let report_name = theme.name;
        for kind in CvdKind::ALL {
            match min_semantic_delta(report_name, kind) {
                Some((delta, left, right)) => {
                    let close: Vec<String> = close_pairs(report_name, kind, CLOSE_DELTA)
                        .into_iter()
                        .map(|pair| {
                            format!(
                                "{}/{} {:.0}",
                                pair.left.as_str(),
                                pair.right.as_str(),
                                pair.delta
                            )
                        })
                        .collect();
                    let close = if close.is_empty() {
                        "none".into()
                    } else {
                        close.join(", ")
                    };
                    lines.push(format!(
                        "  {} min {:.0} ({}/{})  close: {close}",
                        kind.as_str(),
                        delta,
                        left.as_str(),
                        right.as_str()
                    ));
                }
                None => lines.push(format!("  {} n/a", kind.as_str())),
            }
        }
        lines.push(format!(
            "  critical ok/err: deutan {:.0}  protan {:.0}  tritan {:.0}",
            critical_delta(report_name, Role::Ok, Role::Err, CvdKind::Deutan),
            critical_delta(report_name, Role::Ok, Role::Err, CvdKind::Protan),
            critical_delta(report_name, Role::Ok, Role::Err, CvdKind::Tritan)
        ));
        lines.push(format!(
            "  critical warn/err: deutan {:.0}  protan {:.0}  tritan {:.0}",
            critical_delta(report_name, Role::Warn, Role::Err, CvdKind::Deutan),
            critical_delta(report_name, Role::Warn, Role::Err, CvdKind::Protan),
            critical_delta(report_name, Role::Warn, Role::Err, CvdKind::Tritan)
        ));
    }
    lines.join("\n")
}

fn contrast_cell(
    fg: Option<super::theme::Rgb>,
    bg: Option<super::theme::Rgb>,
    ascii: bool,
) -> String {
    match (fg, bg) {
        (Some(fg), Some(bg)) => format!("{:.1}", contrast_ratio(fg, bg)),
        _ => super::glyphs::chrome_dash(ascii).into(),
    }
}

fn render_sample(theme: &Theme) -> String {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("preview backend");
    terminal
        .draw(|frame| draw_sample(frame, theme))
        .expect("preview draw");
    buffer_text(terminal.backend())
}

fn draw_sample(frame: &mut ratatui::Frame, theme: &Theme) {
    let area = frame.area();
    if theme.paints_background {
        frame.render_widget(Block::default().style(theme.bg), area);
    }
    let rows = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Min(1),
    ])
    .split(area);
    let columns = Layout::horizontal([
        Constraint::Length(10),
        Constraint::Min(20),
        Constraint::Length(28),
    ])
    .split(rows[0]);
    let glyphs = theme.glyphs;
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{}Case", glyphs.focus),
                theme.accent.add_modifier(ratatui::style::Modifier::BOLD),
            )),
            Line::from(Span::styled(" Entities", theme.fg)),
            Line::from(Span::styled(" Evidence", theme.fg)),
            Line::from(Span::styled(" Graph", theme.fg)),
            Line::from(Span::styled(
                format!("{}Review 3", glyphs.focus),
                theme.ai.add_modifier(ratatui::style::Modifier::BOLD),
            )),
            Line::from(Span::styled(" Jobs", theme.muted)),
        ])
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme.rule),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("KIND VALUE            PROV SRC", theme.muted)),
            Line::from(vec![
                Span::styled(
                    format!(
                        "{} acme-login.com    ",
                        Glyphs::entity_tag(&crate::model::EntityKind::Domain)
                    ),
                    theme.fg,
                ),
                Span::styled(glyphs.collected, theme.col),
                Span::styled("    6", theme.muted),
            ]),
            Line::from(vec![
                Span::styled(
                    format!(
                        "{} 203.0.113.7       ",
                        Glyphs::entity_tag(&crate::model::EntityKind::Ip)
                    ),
                    theme.fg,
                ),
                Span::styled(glyphs.external, theme.ext),
                Span::styled("    3", theme.muted),
            ]),
            Line::from(vec![Span::styled(
                format!("{} Finding: phishing host", glyphs.ai_pending),
                theme.ai,
            )]),
            Line::from(Span::styled(
                "Accept 1 AI finding? It will keep agent provenance.",
                theme.fg,
            )),
        ])
        .block(Block::default().style(theme.panel)),
        columns[1],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{}Inspector", glyphs.focus),
                theme.accent.add_modifier(ratatui::style::Modifier::BOLD),
            )),
            Line::from(Span::styled("DOM acme-login.com", theme.fg)),
            Line::from(vec![
                Span::styled(glyphs.collected, theme.col),
                Span::styled(
                    format!(" 14 facts{}6 sources", theme.glyphs.sep()),
                    theme.muted,
                ),
            ]),
            Line::from(Span::styled("Will write", theme.warn)),
            Line::from(vec![
                Span::raw("+ Finding  "),
                Span::styled(glyphs.ai, theme.ai),
                Span::styled(" AI", theme.ai),
            ]),
        ]),
        columns[2],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Accept 1 AI finding? ", theme.fg),
            Span::styled("y confirm", theme.accent),
            Span::styled("   any other key cancels", theme.muted),
        ]))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme.rule)
                .style(theme.panel),
        ),
        rows[1],
    );
    let _ = rows;
}

fn buffer_text(backend: &TestBackend) -> String {
    let buffer = backend.buffer();
    let area = buffer.area();
    let mut rows = Vec::new();
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        rows.push(row.trim_end().to_string());
    }
    while rows.last().is_some_and(|row| row.is_empty()) {
        rows.pop();
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_env() -> TerminalEnv {
        TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: false,
            colorfgbg: Some("15;0".into()),
        }
    }

    #[test]
    fn preview_all_matches_committed_snapshot() {
        let mut config = Config::default();
        config.ui.theme = "auto".into();
        config.ui.color = "truecolor".into();
        config.ui.ascii = false;
        let actual = render_preview(&config, &fixture_env(), None, true).expect("preview");
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests/golden/theme_preview_all.txt");
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if expected.is_empty() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("golden dir");
            std::fs::write(&path, &actual).expect("write golden");
            panic!(
                "wrote {} — re-run tests to confirm the snapshot",
                path.display()
            );
        }
        assert_eq!(
            actual, expected,
            "theme preview snapshot drifted; update tests/golden/theme_preview_all.txt if intended"
        );
    }

    #[test]
    fn preview_all_ascii_is_snapshot_tested() {
        let mut config = Config::default();
        config.ui.color = "truecolor".into();
        config.ui.ascii = true;
        let actual = render_preview(&config, &fixture_env(), None, true).expect("preview");
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests/golden/theme_preview_all_ascii.txt");
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if expected.is_empty() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("golden dir");
            std::fs::write(&path, &actual).expect("write golden");
            panic!(
                "wrote {} — re-run tests to confirm the snapshot",
                path.display()
            );
        }
        assert_eq!(actual, expected);
        assert!(
            actual.contains("acme-login.com    C"),
            "ASCII preview must use C for collected"
        );
        assert!(!actual.contains('●'));
        assert!(actual.contains(">? Finding:") || actual.contains("*? Finding:"));
    }

    #[test]
    fn phosphor_preview_at_16_includes_the_hint() {
        let mut config = Config::default();
        config.ui.theme = "phosphor".into();
        config.ui.color = "16".into();
        let env = TerminalEnv {
            colorterm: None,
            term: Some("xterm".into()),
            no_color: false,
            colorfgbg: None,
        };
        let actual = render_preview(&config, &env, Some("phosphor"), false).expect("preview");
        assert!(actual.contains("phosphor needs 256 colours"), "{actual}");
        assert!(actual.contains("Mono"), "{actual}");
    }

    #[test]
    fn preview_unknown_and_retired_names_error() {
        let config = Config::default();
        for name in ["unknown", "default", "high-contrast"] {
            let error = render_preview(&config, &fixture_env(), Some(name), false)
                .expect_err(name)
                .to_string();
            assert!(error.contains("unknown theme"), "{error}");
            assert!(error.contains("dark"), "{error}");
            assert!(error.contains("light"), "{error}");
            assert!(error.contains("mono"), "{error}");
            assert!(error.contains("phosphor"), "{error}");
            assert!(error.contains("amber"), "{error}");
        }
    }
}
