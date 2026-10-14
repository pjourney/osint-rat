//! Small drawing helpers shared by screens and overlays.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use unicode_width::UnicodeWidthStr;

use crate::app::Input;
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
    let border = if focused {
        theme.focused_border
    } else {
        theme.unfocused_border
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border)
        .title_style(if focused { theme.header } else { theme.muted })
}

pub fn truncate(text: &str, max_width: usize) -> String {
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
        Provenance::Agent => theme.badge_ai,
        Provenance::Collected => theme.badge_col,
        Provenance::Analyst | Provenance::External => theme.badge_anl,
    };
    Span::styled(format!("[{}]", provenance.badge()), style)
}

pub fn hint_line(theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("[a]", theme.header),
        Span::raw(" Ask  "),
        Span::styled("[/]", theme.header),
        Span::raw(" Search  "),
        Span::styled("[p]", theme.header),
        Span::raw(" Palette  "),
        Span::styled("[r]", theme.header),
        Span::raw(" Collect  "),
        Span::styled("[g]", theme.header),
        Span::raw(" Graph  "),
        Span::styled("[s]", theme.header),
        Span::raw(" Around  "),
        Span::styled("[?]", theme.header),
        Span::raw(" Help"),
    ])
}
