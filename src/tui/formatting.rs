use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn right_aligned_line(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let left_width = spans_width(&left);
    let right_width = spans_width(&right);
    let padding = width.saturating_sub(left_width + right_width).max(1);
    left.push(Span::raw(" ".repeat(padding)));
    left.extend(right);
    Line::from(left)
}

pub(super) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}

pub(super) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(super) fn album_divider(width: usize) -> String {
    match width {
        0 => String::new(),
        1 => " ".to_string(),
        2 => "  ".to_string(),
        width => format!(" {} ", "-".repeat(width - 2)),
    }
}

pub(super) fn truncate_to_width(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width {
            break;
        }
        out.push(character);
        width += character_width;
    }
    out
}

pub(super) fn fit_to_width(text: &str, width: usize) -> String {
    let mut text = truncate_to_width(text, width);
    let padding = width.saturating_sub(display_width(&text));
    if padding > 0 {
        text.push_str(&" ".repeat(padding));
    }
    text
}

pub(super) fn push_limited_span(
    spans: &mut Vec<Span<'static>>,
    remaining: &mut usize,
    text: &str,
    style: Style,
) {
    if *remaining == 0 {
        return;
    }
    let text = truncate_to_width(text, *remaining);
    *remaining = (*remaining).saturating_sub(display_width(&text));
    spans.push(Span::styled(text, style));
}
