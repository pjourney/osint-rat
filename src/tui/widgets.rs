//! Small drawing helpers shared by screens and overlays.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use unicode_width::UnicodeWidthStr;

use crate::app::commands::context_commands;
use crate::app::{Input, Pane, View};
use crate::model::Provenance;
use crate::sanitize::sanitize_line;

use super::theme::Theme;

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2).max(1));
    let height = height.min(area.height.saturating_sub(2).max(1));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

pub fn pane_block<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let border = if focused { theme.accent } else { theme.rule };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border)
        .title_style(if focused { theme.accent } else { theme.muted })
}

/// Title row for a cockpit pane. Unfocused panes have no box; the focused pane
/// gets a ▌ accent and a bold title.
pub fn pane_title(title: &str, focused: bool, theme: &Theme) -> Line<'static> {
    pane_title_pulse(title, focused, theme, None)
}

pub fn pane_title_pulse(
    title: &str,
    focused: bool,
    theme: &Theme,
    pulse: Option<crate::tui::motion::FrameView>,
) -> Line<'static> {
    if focused {
        Line::from(Span::styled(
            format!("{}{title}", theme.glyphs.focus),
            crate::tui::motion::focus_style(pulse, theme),
        ))
    } else {
        Line::from(Span::styled(format!(" {title}"), theme.muted))
    }
}

pub fn rule_span(theme: &Theme) -> Span<'static> {
    Span::styled("│", theme.rule)
}

pub fn truncate(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let clean = sanitize_line(text);
    if UnicodeWidthStr::width(clean.as_ref()) <= max_width {
        return clean.into_owned();
    }
    let mut output = String::new();
    let mut width = 0usize;
    let limit = max_width.saturating_sub(1);
    for ch in clean.chars() {
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

pub fn input_line(input: &Input) -> Line<'static> {
    let value = sanitize_line(&input.value).into_owned();
    let cursor = input.cursor.min(input.value.len());
    let prefix = sanitize_line(&input.value[..cursor]).into_owned();
    let rest = input.value.get(cursor..).unwrap_or("");
    let mut chars = rest.chars();
    let current = chars
        .next()
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| " ".into());
    let suffix = sanitize_line(chars.as_str()).into_owned();
    let _ = value;
    Line::from(vec![
        Span::raw(prefix),
        Span::styled(current, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(suffix),
    ])
}

pub fn list_from<'a>(items: Vec<String>, selected: usize, theme: &Theme) -> List<'a> {
    let rows: Vec<ListItem> = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let style = if index == selected {
                theme.selected
            } else {
                Style::default()
            };
            ListItem::new(truncate(&item, 120)).style(style)
        })
        .collect();
    List::new(rows)
}

pub fn provenance_style(provenance: Provenance, theme: &Theme) -> Style {
    match provenance {
        Provenance::Agent => theme.ai,
        Provenance::Collected => theme.col,
        Provenance::External => theme.ext,
        Provenance::Analyst => theme.anl,
    }
}

pub fn provenance_glyph(provenance: Provenance, theme: &Theme) -> Span<'static> {
    Span::styled(
        theme.glyphs.provenance(provenance).to_string(),
        provenance_style(provenance, theme),
    )
}

pub fn provenance_span(provenance: Provenance, theme: &Theme) -> Span<'static> {
    let glyph = theme.glyphs.provenance(provenance);
    Span::styled(
        format!("{glyph} {}", provenance.badge()),
        provenance_style(provenance, theme),
    )
}

/// Shared empty board: a glyph, a sentence, and the keys that populate it. Not a selectable row.
pub fn empty_state(title: &str, body: &str, actions: &str, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("            {}  {title}", theme.glyphs.external),
            theme.muted,
        )),
        Line::from(Span::styled(format!("            {body}"), theme.muted)),
        Line::from(Span::styled(format!("            {actions}"), theme.accent)),
    ]
}

/// Skeleton rows while a job is running. Not selectable.
pub fn loading_state(job_ref: &str, label: &str, theme: &Theme) -> Vec<Line<'static>> {
    loading_state_spin(job_ref, label, theme, theme.glyphs.spinner[0])
}

pub fn loading_state_spin(
    job_ref: &str,
    label: &str,
    theme: &Theme,
    spin: &str,
) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("  {spin} {job_ref}  {label}"),
            theme.muted,
        )),
        Line::from(Span::styled("  ░░░░░░░░░░░░░░░░░░░░", theme.muted)),
        Line::from(Span::styled("  ░░░░░░░░░░░░░░░░░░░░", theme.muted)),
    ]
}

/// Inline error panel. Stays until the analyst dismisses it with `x`.
pub fn error_panel(message: &str, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("  {}  {}", theme.glyphs.failed, sanitize_line(message)),
            theme.err,
        )),
        Line::from(Span::styled(
            "     Check the cause, then Ctrl+K doctor.  x dismiss",
            theme.muted,
        )),
    ]
}

pub fn context_line(view: View, pane: Pane, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for command in context_commands(view, pane) {
        if command.hint.is_empty() || !seen.insert(command.hint) {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(command.hint.to_string(), theme.accent));
        spans.push(Span::raw(format!(" {}", hint_label(command, view))));
    }
    Line::from(spans)
}

fn hint_label(command: &crate::app::commands::Command, view: View) -> String {
    if command.id == "search" && crate::app::App::view_uses_table_filter(view) {
        return "filter".into();
    }
    match command.title {
        "Ask Agent" => "ask agent".into(),
        "Command palette" => "commands".into(),
        "Run Collectors" => "collect".into(),
        "Search Around" => "search around".into(),
        "Open selection in Graph" => "graph".into(),
        "Select for accept" => "arm".into(),
        "Time lens" => "time".into(),
        other => other.to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_width_zero_emits_nothing() {
        assert_eq!(truncate("hello", 0), "");
        assert_eq!(truncate("café", 0), "");
    }

    #[test]
    fn truncate_cjk_and_emoji_use_display_width() {
        use unicode_width::UnicodeWidthStr;
        let cjk = truncate("你好世界", 5);
        assert!(UnicodeWidthStr::width(cjk.as_str()) <= 5, "{cjk}");
        assert!(cjk.ends_with('…'), "{cjk}");
        let emoji = truncate("ok👍ok", 5);
        assert!(UnicodeWidthStr::width(emoji.as_str()) <= 5, "{emoji}");
        assert!(emoji.contains('…') || UnicodeWidthStr::width(emoji.as_str()) <= 5);
    }
}
