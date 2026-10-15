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
    if focused {
        Line::from(Span::styled(
            format!("{}{title}", theme.glyphs.focus),
            theme.accent.add_modifier(Modifier::BOLD),
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

pub fn provenance_span(provenance: Provenance, theme: &Theme) -> Span<'static> {
    let style = match provenance {
        Provenance::Agent => theme.ai,
        Provenance::Collected => theme.col,
        Provenance::External => theme.ext,
        Provenance::Analyst => theme.anl,
    };
    let glyph = theme.glyphs.provenance(provenance);
    Span::styled(format!("{glyph} {}", provenance.badge()), style)
}

pub fn context_line(view: View, pane: Pane, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, command) in context_commands(view, pane).into_iter().enumerate() {
        if command.hint.is_empty() {
            continue;
        }
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(command.hint.to_string(), theme.accent));
        spans.push(Span::raw(format!(" {}", short_hint_label(command.title))));
    }
    Line::from(spans)
}

fn short_hint_label(title: &str) -> String {
    match title {
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
}
