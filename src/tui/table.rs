//! Width-aware tables: headers, gutters, sort markers, display-width truncation.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::filter::SortKey;
use crate::model::{Provenance, Timestamp};

use super::theme::Theme;
use super::widgets::{ellipsize, provenance_glyph};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub key: SortKey,
    pub header: &'static str,
    pub min: u16,
    pub weight: u16,
    pub align: Align,
}

#[derive(Clone, Debug)]
pub struct TableCell {
    pub spans: Vec<Span<'static>>,
}

impl TableCell {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            spans: vec![Span::raw(text.into())],
        }
    }

    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            spans: vec![Span::styled(text.into(), style)],
        }
    }

    pub fn spans(spans: Vec<Span<'static>>) -> Self {
        Self { spans }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_table(
    frame: &mut Frame,
    area: Rect,
    columns: &[Column],
    rows: &[Vec<TableCell>],
    selected: usize,
    sort: Option<(SortKey, bool)>,
    focused: bool,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let widths = column_widths(columns, area.width);
    let mut lines = vec![header_line(columns, &widths, sort, theme)];
    let body = area.height.saturating_sub(1) as usize;
    let start = selected.saturating_sub(body.saturating_sub(1));
    for (index, row) in rows.iter().enumerate().skip(start).take(body) {
        let style = if index == selected {
            theme.selected
        } else {
            theme.fg
        };
        lines.push(row_line(
            columns,
            &widths,
            row,
            style,
            focused && index == selected,
            theme,
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub fn column_widths(columns: &[Column], total: u16) -> Vec<u16> {
    if columns.is_empty() {
        return Vec::new();
    }
    let gutters = 2 * columns.len().saturating_sub(1);
    let min_sum: u16 = columns.iter().map(|column| column.min).sum();
    let mut widths: Vec<u16> = columns.iter().map(|column| column.min).collect();
    let available = total.saturating_sub(min_sum.saturating_add(gutters as u16));
    let weight_sum: u16 = columns.iter().map(|column| column.weight).sum();
    if available == 0 || weight_sum == 0 {
        return widths;
    }
    let mut used = 0u16;
    let last_flex = columns
        .iter()
        .rposition(|column| column.weight > 0)
        .unwrap_or(columns.len() - 1);
    for (index, column) in columns.iter().enumerate() {
        if column.weight == 0 {
            continue;
        }
        let share = if index == last_flex {
            available.saturating_sub(used)
        } else {
            available * column.weight / weight_sum
        };
        widths[index] = widths[index].saturating_add(share);
        used = used.saturating_add(share);
    }
    widths
}

pub fn relative_time(timestamp: Timestamp) -> String {
    relative_time_from(timestamp, crate::model::now())
}

pub fn relative_time_from(timestamp: Timestamp, now: Timestamp) -> String {
    let delta = now.signed_duration_since(timestamp);
    let secs = delta.num_seconds().abs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

pub fn provenance_cells(provenances: &[Provenance], theme: &Theme) -> TableCell {
    if provenances.is_empty() {
        return TableCell::styled("—", theme.muted);
    }
    let spans = provenances
        .iter()
        .copied()
        .map(|provenance| provenance_glyph(provenance, theme))
        .collect();
    TableCell::spans(spans)
}

pub fn sort_marker(descending: bool, ascii: bool) -> &'static str {
    if descending {
        if ascii { "v" } else { "▼" }
    } else if ascii {
        "^"
    } else {
        "▲"
    }
}

fn header_line(
    columns: &[Column],
    widths: &[u16],
    sort: Option<(SortKey, bool)>,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let width = widths.get(index).copied().unwrap_or(column.min) as usize;
        let marker = match sort {
            Some((key, descending)) if column.key == key => {
                sort_marker(descending, theme.glyphs.ascii)
            }
            _ => "",
        };
        let label = format!("{}{marker}", column.header);
        let cell = pad_align(&label, width, column.align, theme.glyphs.ascii);
        spans.push(Span::styled(cell, theme.muted));
    }
    Line::from(spans)
}

fn row_line(
    columns: &[Column],
    widths: &[u16],
    row: &[TableCell],
    style: Style,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let width = widths.get(index).copied().unwrap_or(column.min) as usize;
        let cell = row
            .get(index)
            .cloned()
            .unwrap_or_else(|| TableCell::text(""));
        spans.extend(fit_cell(
            &cell,
            width,
            column.align,
            style,
            selected && index == 0,
            theme.glyphs.ascii,
        ));
    }
    Line::from(spans)
}

fn fit_cell(
    cell: &TableCell,
    width: usize,
    align: Align,
    style: Style,
    _mark_selected: bool,
    ascii: bool,
) -> Vec<Span<'static>> {
    let raw: String = cell
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut text = if UnicodeWidthStr::width(raw.as_str()) > width {
        ellipsize(&raw, width, ascii)
    } else {
        raw
    };
    text = pad_align(&text, width, align, ascii);
    if cell.spans.len() == 1 {
        let span_style = cell.spans[0].style.patch(style);
        return vec![Span::styled(text, span_style)];
    }
    if cell.spans.is_empty() {
        return vec![Span::styled(text, style)];
    }
    let mut spans = Vec::new();
    let mut used = 0usize;
    let total: usize = cell
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let mark = if ascii { "..." } else { "…" };
    let mark_width = if ascii { 3 } else { 1 };
    let ellipsis = total > width;
    let budget = if ellipsis {
        width.saturating_sub(mark_width)
    } else {
        width
    };
    for span in &cell.spans {
        let piece = span.content.as_ref();
        let piece_width = UnicodeWidthStr::width(piece);
        if used + piece_width > budget {
            let remain = budget.saturating_sub(used);
            if remain > 0 {
                spans.push(Span::styled(
                    take_width(piece, remain),
                    span.style.patch(style),
                ));
                used += remain;
            }
            break;
        }
        spans.push(Span::styled(piece.to_string(), span.style.patch(style)));
        used += piece_width;
    }
    if ellipsis {
        let shown = if width < mark_width {
            mark.chars().take(width).collect::<String>()
        } else {
            mark.to_string()
        };
        spans.push(Span::styled(shown.clone(), style));
        used = used.saturating_add(UnicodeWidthStr::width(shown.as_str()));
    }
    if used < width {
        let pad = " ".repeat(width - used);
        match align {
            Align::Left => spans.push(Span::raw(pad)),
            Align::Right => spans.insert(0, Span::raw(pad)),
        }
    }
    spans
}

fn take_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width + ch_width > max_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output
}

fn pad_align(text: &str, width: usize, align: Align, ascii: bool) -> String {
    let current = UnicodeWidthStr::width(text);
    if current >= width {
        return ellipsize(text, width, ascii);
    }
    let pad = " ".repeat(width - current);
    match align {
        Align::Left => format!("{text}{pad}"),
        Align::Right => format!("{pad}{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn flex_column_takes_remaining_width() {
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
                key: SortKey::Src,
                header: "SRC",
                min: 3,
                weight: 0,
                align: Align::Right,
            },
        ];
        let widths = column_widths(&columns, 40);
        assert_eq!(widths[0], 4);
        assert_eq!(widths[2], 3);
        assert_eq!(widths[1], 40 - 4 - 3 - 4);
    }

    #[test]
    fn relative_time_uses_largest_unit() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 15, 12, 0, 0)
            .single()
            .expect("timestamp");
        let hour = now - chrono::Duration::minutes(90);
        assert_eq!(relative_time_from(hour, now), "1h");
        let day = now - chrono::Duration::hours(50);
        assert_eq!(relative_time_from(day, now), "2d");
    }

    #[test]
    fn pad_align_respects_display_width() {
        assert_eq!(pad_align("ab", 4, Align::Left, false), "ab  ");
        assert_eq!(pad_align("12", 4, Align::Right, false), "  12");
        assert_eq!(
            UnicodeWidthStr::width(pad_align("café", 6, Align::Left, false).as_str()),
            6
        );
    }

    #[test]
    fn multi_span_cells_ellipsis_when_truncated() {
        let cell = TableCell::spans(vec![
            ratatui::text::Span::raw("●"),
            ratatui::text::Span::raw("◇"),
            ratatui::text::Span::raw("✎"),
            ratatui::text::Span::raw("◆"),
        ]);
        let spans = fit_cell(
            &cell,
            2,
            Align::Left,
            ratatui::style::Style::default(),
            false,
            false,
        );
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert!(text.contains('…'), "{text}");
        assert!(UnicodeWidthStr::width(text.trim_end()) <= 2, "{text}");
    }
}
