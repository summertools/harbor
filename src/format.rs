//! Formatting and text-safety helpers shared by the CLI and TUI.

use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Human-readable binary byte count.
#[must_use]
pub fn bytes(value: u64) -> String {
    if value < 1024 {
        return format!("{value} B");
    }
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut scaled = value as f64;
    let mut index = 0usize;
    while scaled >= 1024.0 && index + 1 < units.len() {
        scaled /= 1024.0;
        index += 1;
    }
    if scaled >= 100.0 {
        format!("{scaled:.0} {}", units[index])
    } else if scaled >= 10.0 {
        format!("{scaled:.1} {}", units[index])
    } else {
        format!("{scaled:.2} {}", units[index])
    }
}

/// Compact byte count for dense tables, for example `9.4M`.
#[must_use]
pub fn bytes_short(value: u64) -> String {
    if value < 1024 {
        return format!("{value}B");
    }
    let units = ["B", "K", "M", "G", "T", "P"];
    let mut scaled = value as f64;
    let mut index = 0usize;
    while scaled >= 1024.0 && index + 1 < units.len() {
        scaled /= 1024.0;
        index += 1;
    }
    if scaled >= 10.0 {
        format!("{scaled:.0}{}", units[index])
    } else {
        format!("{scaled:.1}{}", units[index])
    }
}

/// Display width in terminal cells.
#[must_use]
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncate to `max_width` cells, appending an ellipsis when shortened.
#[must_use]
pub fn truncate(text: &str, max_width: usize) -> Cow<'_, str> {
    if width(text) <= max_width {
        return Cow::Borrowed(text);
    }
    if max_width == 0 {
        return Cow::Borrowed("");
    }
    let budget = max_width.saturating_sub(1);
    let mut out = String::with_capacity(text.len().min(max_width * 4));
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    Cow::Owned(out)
}

/// Left-truncate a path-like string while retaining its final components.
#[must_use]
pub fn truncate_left(text: &str, max_width: usize) -> Cow<'_, str> {
    if width(text) <= max_width {
        return Cow::Borrowed(text);
    }
    if max_width == 0 {
        return Cow::Borrowed("");
    }
    let budget = max_width.saturating_sub(1);
    let mut reversed = Vec::new();
    let mut used = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        reversed.push(ch);
        used += ch_width;
    }
    let mut out = String::with_capacity(text.len().min(max_width * 4));
    out.push('…');
    out.extend(reversed.into_iter().rev());
    Cow::Owned(out)
}

/// Pad to a display width.
#[must_use]
pub fn pad_right(text: &str, target: usize) -> String {
    let used = width(text);
    if used >= target {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + target - used);
    out.push_str(text);
    out.extend(std::iter::repeat_n(' ', target - used));
    out
}

/// Pad on the left to a display width.
#[must_use]
pub fn pad_left(text: &str, target: usize) -> String {
    let used = width(text);
    if used >= target {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + target - used);
    out.extend(std::iter::repeat_n(' ', target - used));
    out.push_str(text);
    out
}

/// Remove terminal escape, bidi, and zero-width control characters from
/// untrusted text before it reaches a width calculation or a terminal.
#[must_use]
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\t' | '\n' | '\r' | '\u{2028}' | '\u{2029}' => out.push(' '),
            '\u{200B}'..='\u{200D}' | '\u{FEFF}' | '\u{00AD}' => {}
            '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}' => out.push('?'),
            ch if ch.is_control() => out.push('?'),
            ch => out.push(ch),
        }
    }
    out
}

/// Sanitise and cap the number of characters.
#[must_use]
pub fn sanitize_limited(input: &str, max_chars: usize) -> String {
    let clean = sanitize(input);
    if clean.chars().count() <= max_chars {
        clean
    } else {
        clean.chars().take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_compact_and_readable() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1024), "1.00 KiB");
        assert_eq!(bytes(10 * 1024 * 1024), "10.0 MiB");
    }

    #[test]
    fn truncation_counts_wide_characters() {
        assert_eq!(truncate("abc日本語", 5), "abc…");
        assert_eq!(truncate_left("/home/user/日本語", 7), "…日本語");
    }

    #[test]
    fn sanitize_removes_escape_and_bidi_controls() {
        let dirty = "x\u{1b}[31m\u{202e}\u{200b}y";
        let clean = sanitize(dirty);
        assert!(!clean.contains('\u{1b}'));
        assert!(!clean.contains('\u{202e}'));
        assert!(!clean.contains('\u{200b}'));
        assert!(clean.starts_with('x'));
    }
}
