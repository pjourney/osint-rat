//! Sanitization of untrusted text before it is stored, rendered, or shown to an agent.
//!
//! Web pages, DNS TXT records, tool output, and agent responses are untrusted. Raw control
//! characters or ANSI escape sequences in that content could otherwise manipulate the user's
//! terminal (cursor movement, title changes, OSC 52 clipboard writes, hyperlinks, ...).
//! Bidirectional override characters are removed as well so text cannot be visually reordered.

use std::borrow::Cow;

const ESC: char = '\u{1b}';
const BEL: char = '\u{07}';
const TAB_REPLACEMENT: &str = "    ";
/// The 8-bit "C1" control range, which some terminals interpret like ESC sequences.
const C1_START: char = '\u{80}';
const C1_END: char = '\u{9f}';
const CSI_8BIT: char = '\u{9b}';
const OSC_8BIT: char = '\u{9d}';
const STRING_TERMINATOR_8BIT: char = '\u{9c}';

/// Sanitizes multi-line text: strips escape sequences and control characters, normalizes line
/// endings to `\n`, expands tabs, and removes bidirectional overrides.
pub fn sanitize_text(input: &str) -> Cow<'_, str> {
    if is_already_clean(input, true) {
        return Cow::Borrowed(input);
    }
    Cow::Owned(clean(input, true))
}

/// Sanitizes text that must stay on a single line (titles, table cells, header values).
pub fn sanitize_line(input: &str) -> Cow<'_, str> {
    if is_already_clean(input, false) {
        return Cow::Borrowed(input);
    }
    Cow::Owned(clean(input, false))
}

/// Decodes bytes as UTF-8 (lossily) and sanitizes the result.
pub fn sanitize_bytes(bytes: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    sanitize_text(&decoded).into_owned()
}

/// Truncates `input` to at most `max_bytes` bytes on a character boundary, appending a marker
/// that states how much was omitted.
pub fn truncate_with_marker(input: &str, max_bytes: usize) -> Cow<'_, str> {
    if input.len() <= max_bytes {
        return Cow::Borrowed(input);
    }
    let cut = floor_char_boundary(input, max_bytes);
    let omitted = input.len() - cut;
    Cow::Owned(format!(
        "{}\n[… truncated: {} omitted]",
        &input[..cut],
        format_bytes(omitted)
    ))
}

/// Returns the largest index `<= index` that lies on a UTF-8 character boundary.
pub fn floor_char_boundary(input: &str, index: usize) -> usize {
    if index >= input.len() {
        return input.len();
    }
    let mut cut = index;
    while cut > 0 && !input.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Formats a byte count for humans (`512 B`, `12.3 KB`, `4.0 MB`).
pub fn format_bytes(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let value = bytes as f64;
    if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn is_already_clean(input: &str, allow_newlines: bool) -> bool {
    input.chars().all(|c| {
        if c == '\n' {
            return allow_newlines;
        }
        !c.is_control() && !is_bidi_control(c)
    })
}

fn clean(input: &str, allow_newlines: bool) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ESC => skip_escape_sequence(&mut chars),
            CSI_8BIT => skip_csi_body(&mut chars),
            OSC_8BIT => skip_string_body(&mut chars),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push_line_break(&mut output, allow_newlines);
            }
            '\n' => push_line_break(&mut output, allow_newlines),
            '\t' => output.push_str(TAB_REPLACEMENT),
            c if is_bidi_control(c) => {}
            c if (C1_START..=C1_END).contains(&c) => {}
            c if c.is_control() => {}
            c => output.push(c),
        }
    }
    output
}

fn push_line_break(output: &mut String, allow_newlines: bool) {
    if allow_newlines {
        output.push('\n');
    } else if !output.ends_with(' ') {
        output.push(' ');
    }
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => skip_csi_body(chars),
        Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => skip_string_body(chars),
        Some('(') | Some(')') | Some('*') | Some('+') | Some('#') | Some('%') => {
            chars.next();
        }
        _ => {}
    }
}

/// CSI sequences end with a byte in the range `@`..=`~`.
fn skip_csi_body(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for c in chars.by_ref() {
        if ('@'..='~').contains(&c) {
            break;
        }
    }
}

/// OSC/DCS/APC/PM/SOS strings end with BEL, ST (ESC `\`), or the 8-bit ST.
fn skip_string_body(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(c) = chars.next() {
        if c == BEL || c == STRING_TERMINATOR_8BIT {
            break;
        }
        if c == ESC {
            if chars.peek() == Some(&'\\') {
                chars.next();
            }
            break;
        }
    }
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_is_borrowed() {
        let input = "plain text\nwith lines";
        let actual = sanitize_text(input);
        assert!(matches!(actual, Cow::Borrowed(_)));
    }

    #[test]
    fn strips_sgr_color_sequences() {
        let input = "\u{1b}[31mred\u{1b}[0m text";
        let actual = sanitize_text(input);
        assert_eq!(actual, "red text");
    }

    #[test]
    fn strips_osc_title_and_hyperlinks() {
        let input_title = "\u{1b}]0;pwned\u{07}after";
        let input_link = "\u{1b}]8;;https://evil.example\u{1b}\\link\u{1b}]8;;\u{1b}\\";
        assert_eq!(sanitize_text(input_title), "after");
        assert_eq!(sanitize_text(input_link), "link");
    }

    #[test]
    fn strips_osc52_clipboard_write() {
        let input = "a\u{1b}]52;c;ZWNobyBwd25lZA==\u{07}b";
        assert_eq!(sanitize_text(input), "ab");
    }

    #[test]
    fn strips_eight_bit_c1_sequences() {
        let input = "x\u{9b}2Jy\u{9d}0;t\u{9c}z";
        assert_eq!(sanitize_text(input), "xyz");
    }

    #[test]
    fn removes_bidi_overrides_and_controls() {
        let input = "abc\u{202e}fed\u{0}\u{7}\u{8}";
        assert_eq!(sanitize_text(input), "abcfed");
    }

    #[test]
    fn normalizes_line_endings_and_tabs() {
        let input = "a\r\nb\rc\td";
        assert_eq!(sanitize_text(input), "a\nb\nc    d");
    }

    #[test]
    fn single_line_collapses_newlines() {
        let input = "Title\nline two\r\nthree";
        assert_eq!(sanitize_line(input), "Title line two three");
    }

    #[test]
    fn unterminated_sequences_do_not_leak() {
        let input = "safe\u{1b}]0;never terminated";
        assert_eq!(sanitize_text(input), "safe");
    }

    #[test]
    fn sanitize_bytes_handles_invalid_utf8() {
        let input: &[u8] = &[b'o', b'k', 0xff, 0x1b, b'[', b'1', b'm'];
        assert_eq!(sanitize_bytes(input), "ok\u{fffd}");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let input = "ééééé";
        let actual = truncate_with_marker(input, 3);
        assert!(actual.starts_with('é'));
        assert!(actual.contains("truncated"));
    }

    #[test]
    fn format_bytes_uses_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
