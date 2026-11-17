//! CommonMark 0.31.2 subset parser and HTML renderer.
//!
//! Supported block constructs:
//! - Thematic breaks (4.1)
//! - ATX headings `#`..`######` (4.2)
//! - Fenced code blocks with ` ``` ` or `~~~` and optional info string rendered as `language-<info>` (4.5)
//! - Paragraphs (4.8)
//! - Block quotes `>` up to nesting depth 32 (5.1)
//! - Tight bullet (`-`, `*`, `+`) and ordered (`1.`, `1)`) lists (5.2, 5.3)
//!
//! Supported inline constructs:
//! - Code spans with backticks (6.1)
//! - Emphasis and strong with `*` and `_`, without intraword `_` emphasis (6.2 simplified)
//! - Inline links `[text](url "title")` (6.3)
//! - Autolinks `<https://...>` and `<mailto:...>` (6.5)
//! - Hard line breaks with two or more trailing spaces before newline (6.7)
//! - Raw HTML is always escaped (`&`, `<`, `>`, `"`, `'`).

use std::fmt::Write;

const MAX_BLOCKQUOTE_DEPTH: usize = 32;

/// Escapes special HTML characters `&`, `<`, `>`, `"`, and `'`.
#[must_use]
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes HTML characters for `CommonMark` text/code (`&`, `<`, `>`, `"`).
fn escape_cm_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Converts a Markdown document into HTML according to the crate's `CommonMark` subset.
#[must_use]
pub fn markdown_to_html(src: &str) -> String {
    let normalized = src.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    render_blocks(&lines, 0)
}

fn render_fenced_code(lines: &[&str], i: &mut usize, out: &mut String) -> bool {
    let line = lines[*i];
    let Some((fence_char, fence_len, info)) = parse_fence_open(line) else {
        return false;
    };
    *i += 1;
    let n = lines.len();
    let mut code_lines = Vec::new();
    while *i < n {
        if is_fence_close(lines[*i], fence_char, fence_len) {
            *i += 1;
            break;
        }
        code_lines.push(lines[*i]);
        *i += 1;
    }
    if info.is_empty() {
        out.push_str("<pre><code>");
    } else {
        let lang = escape_cm_html(info);
        out.push_str("<pre><code class=\"language-");
        out.push_str(&lang);
        out.push_str("\">");
    }
    for cl in &code_lines {
        out.push_str(&escape_cm_html(cl));
        out.push('\n');
    }
    out.push_str("</code></pre>\n");
    true
}

fn render_list_block(lines: &[&str], i: &mut usize, out: &mut String) -> bool {
    let line = lines[*i];
    let Some((list_kind, first_item)) = parse_list_item(line) else {
        return false;
    };
    let n = lines.len();
    let mut items = vec![vec![first_item]];
    *i += 1;
    while *i < n {
        let cur = lines[*i];
        if cur.trim().is_empty()
            || is_thematic_break(cur)
            || parse_fence_open(cur).is_some()
            || parse_atx_heading(cur).is_some()
        {
            break;
        }
        if let Some((k, item_text)) = parse_list_item(cur) {
            if k == list_kind {
                items.push(vec![item_text]);
                *i += 1;
                continue;
            }
            break;
        }
        // Continuation line if indented by at least 2 spaces
        if let Some(cont) = cur.strip_prefix("  ") {
            if let Some(last) = items.last_mut() {
                last.push(cont.trim_start());
            }
            *i += 1;
        } else {
            break;
        }
    }
    match list_kind {
        ListKind::Bullet(_) => out.push_str("<ul>\n"),
        ListKind::Ordered(_, start) => {
            if start == 1 {
                out.push_str("<ol>\n");
            } else {
                let _ = writeln!(out, "<ol start=\"{start}\">");
            }
        }
    }
    for item_lines in items {
        let joined = item_lines.join("\n");
        let rendered = render_inlines(&joined);
        let _ = writeln!(out, "<li>{rendered}</li>");
    }
    match list_kind {
        ListKind::Bullet(_) => out.push_str("</ul>\n"),
        ListKind::Ordered(_, _) => out.push_str("</ol>\n"),
    }
    true
}

fn render_blocks(lines: &[&str], quote_depth: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    let n = lines.len();

    while i < n {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // 1. Fenced code block
        if render_fenced_code(lines, &mut i, &mut out) {
            continue;
        }

        // 2. Thematic break
        if is_thematic_break(line) {
            out.push_str("<hr />\n");
            i += 1;
            continue;
        }

        // 3. ATX Heading
        if let Some((level, content)) = parse_atx_heading(line) {
            let rendered = render_inlines(content);
            let _ = writeln!(out, "<h{level}>{rendered}</h{level}>");
            i += 1;
            continue;
        }

        // 4. Blockquote (up to MAX_BLOCKQUOTE_DEPTH)
        if quote_depth < MAX_BLOCKQUOTE_DEPTH
            && let Some(stripped) = strip_blockquote_marker(line)
        {
            let mut bq_lines = vec![stripped];
            i += 1;
            while i < n {
                if let Some(s) = strip_blockquote_marker(lines[i]) {
                    bq_lines.push(s);
                    i += 1;
                } else {
                    break;
                }
            }
            let inner = render_blocks(&bq_lines, quote_depth + 1);
            out.push_str("<blockquote>\n");
            out.push_str(&inner);
            out.push_str("</blockquote>\n");
            continue;
        }

        // 5. Lists (tight bullet and ordered)
        if render_list_block(lines, &mut i, &mut out) {
            continue;
        }

        // 6. Paragraph
        let mut para_lines = vec![line];
        i += 1;
        while i < n {
            let cur = lines[i];
            if cur.trim().is_empty()
                || parse_fence_open(cur).is_some()
                || is_thematic_break(cur)
                || parse_atx_heading(cur).is_some()
                || (quote_depth < MAX_BLOCKQUOTE_DEPTH && strip_blockquote_marker(cur).is_some())
                || parse_list_item(cur).is_some()
            {
                break;
            }
            para_lines.push(cur);
            i += 1;
        }
        let joined = para_lines.join("\n");
        let rendered = render_inlines(joined.trim_start());
        let _ = writeln!(out, "<p>{rendered}</p>");
    }

    out
}

fn trim_up_to_3_spaces(line: &str) -> &str {
    let mut count = 0;
    for b in line.bytes() {
        if b == b' ' && count < 3 {
            count += 1;
        } else {
            break;
        }
    }
    &line[count..]
}

fn parse_fence_open(line: &str) -> Option<(char, usize, &str)> {
    let s = trim_up_to_3_spaces(line);
    let first = s.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let fence_len = s.chars().take_while(|&c| c == first).count();
    if fence_len < 3 {
        return None;
    }
    let rest = &s[fence_len..];
    if first == '`' && rest.contains('`') {
        return None;
    }
    let info = rest.split_whitespace().next().unwrap_or("");
    Some((first, fence_len, info))
}

fn is_fence_close(line: &str, fence_char: char, min_len: usize) -> bool {
    let s = trim_up_to_3_spaces(line);
    let count = s.chars().take_while(|&c| c == fence_char).count();
    if count < min_len {
        return false;
    }
    s[count..].trim().is_empty()
}

fn is_thematic_break(line: &str) -> bool {
    let s = trim_up_to_3_spaces(line).trim_end();
    let mut marker: Option<char> = None;
    let mut count = 0usize;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' {
            continue;
        }
        if ch == '-' || ch == '*' || ch == '_' {
            if let Some(m) = marker {
                if m != ch {
                    return false;
                }
            } else {
                marker = Some(ch);
            }
            count += 1;
        } else {
            return false;
        }
    }
    count >= 3
}

fn parse_atx_heading(line: &str) -> Option<(usize, &str)> {
    let s = trim_up_to_3_spaces(line);
    let hashes = s.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &s[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let mut content = rest.trim();
    // Strip optional closing hashes preceded by whitespace
    let trimmed_end = content.trim_end_matches('#');
    if trimmed_end.is_empty() || trimmed_end.ends_with(' ') || trimmed_end.ends_with('\t') {
        content = trimmed_end.trim();
    }
    Some((hashes, content))
}

fn strip_blockquote_marker(line: &str) -> Option<&str> {
    let s = trim_up_to_3_spaces(line);
    if let Some(rest) = s.strip_prefix('>') {
        if let Some(after_space) = rest.strip_prefix(' ') {
            Some(after_space)
        } else {
            Some(rest)
        }
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Bullet(char),
    Ordered(char, u32),
}

impl PartialEq for ListKind {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Bullet(a), Self::Bullet(b)) => a == b,
            (Self::Ordered(d1, _), Self::Ordered(d2, _)) => d1 == d2,
            _ => false,
        }
    }
}

fn parse_list_item(line: &str) -> Option<(ListKind, &str)> {
    let s = trim_up_to_3_spaces(line);
    let first = s.chars().next()?;
    if first == '-' || first == '*' || first == '+' {
        let rest = &s[1..];
        if let Some(stripped) = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t')) {
            return Some((ListKind::Bullet(first), stripped.trim_start()));
        }
        return None;
    }
    let digit_count = s.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digit_count) {
        let num_str = &s[..digit_count];
        let num: u32 = num_str.parse().ok()?;
        let after_digits = &s[digit_count..];
        let delim = after_digits.chars().next()?;
        if delim == '.' || delim == ')' {
            let rest = &after_digits[1..];
            if let Some(stripped) = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t')) {
                return Some((ListKind::Ordered(delim, num), stripped.trim_start()));
            }
        }
    }
    None
}

fn render_inlines(text: &str) -> String {
    // Handle hard line breaks (2+ trailing spaces before '\n') and normal line breaks first by splitting lines
    let raw_lines: Vec<&str> = text.split('\n').collect();
    let mut processed = String::with_capacity(text.len());
    for (idx, line) in raw_lines.iter().enumerate() {
        if idx + 1 < raw_lines.len() {
            let trailing_spaces = line.chars().rev().take_while(|&c| c == ' ').count();
            if trailing_spaces >= 2 {
                let trimmed = line[..line.len() - trailing_spaces].trim_end();
                processed.push_str(trimmed);
                processed.push('\x00'); // Sentinel for <br />
            } else {
                processed.push_str(line.trim_end());
                processed.push('\n');
            }
        } else {
            processed.push_str(line);
        }
    }

    render_inline_slice(&processed)
}

fn render_inline_slice(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Hard break sentinel
        if b == 0 {
            out.push_str("<br />\n");
            i += 1;
            continue;
        }

        // Backslash escape of ASCII punctuation
        if b == b'\\' && i + 1 < len && is_ascii_punctuation(bytes[i + 1]) {
            let ch = bytes[i + 1] as char;
            out.push_str(&escape_cm_html(&ch.to_string()));
            i += 2;
            continue;
        }

        // Code span (6.1)
        if b == b'`' {
            let tick_len = s[i..].bytes().take_while(|&c| c == b'`').count();
            let after_open = i + tick_len;
            if let Some((content, next_idx)) = find_closing_ticks(s, after_open, tick_len) {
                let normalized_code = normalize_code_span(content);
                out.push_str("<code>");
                out.push_str(&escape_cm_html(&normalized_code));
                out.push_str("</code>");
                i = next_idx;
                continue;
            }
            for _ in 0..tick_len {
                out.push('`');
            }
            i = after_open;
            continue;
        }

        // Autolink (6.5)
        if b == b'<'
            && let Some((html, next_idx)) = try_parse_autolink(s, i)
        {
            out.push_str(&html);
            i = next_idx;
            continue;
        }

        // Inline link (6.3)
        if b == b'['
            && let Some((label, url, title, next_idx)) = try_parse_inline_link(s, i)
        {
            let rendered_label = render_inline_slice(label);
            let escaped_url = escape_cm_html(url);
            out.push_str("<a href=\"");
            out.push_str(&escaped_url);
            out.push('"');
            if let Some(t) = title {
                out.push_str(" title=\"");
                out.push_str(&escape_cm_html(t));
                out.push('"');
            }
            out.push('>');
            out.push_str(&rendered_label);
            out.push_str("</a>");
            i = next_idx;
            continue;
        }

        // Emphasis / strong with * or _ (6.2 simplified)
        if (b == b'*' || b == b'_')
            && let Some((tag, inner, next_idx)) = try_parse_emphasis(s, i)
        {
            out.push('<');
            out.push_str(tag);
            out.push('>');
            out.push_str(&render_inline_slice(inner));
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
            i = next_idx;
            continue;
        }

        // Regular character (UTF-8 safe)
        let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
        i += ch.len_utf8();
    }

    out
}

fn is_ascii_punctuation(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'"'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b'-'
            | b'.'
            | b'/'
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'@'
            | b'['
            | b'\\'
            | b']'
            | b'^'
            | b'_'
            | b'`'
            | b'{'
            | b'|'
            | b'}'
            | b'~'
    )
}

fn find_closing_ticks(s: &str, start: usize, tick_len: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = start;
    while i < len {
        if bytes[i] == b'`' {
            let run = s[i..].bytes().take_while(|&c| c == b'`').count();
            if run == tick_len {
                return Some((&s[start..i], i + run));
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

fn normalize_code_span(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| if c == '\n' || c == '\x00' { ' ' } else { c })
        .collect();
    if s.len() >= 2 && s.starts_with(' ') && s.ends_with(' ') && s.chars().any(|c| c != ' ') {
        s.remove(0);
        s.pop();
    }
    s
}

fn try_parse_autolink(s: &str, start: usize) -> Option<(String, usize)> {
    let rest = &s[start + 1..];
    let close_rel = rest.find('>')?;
    let inner = &rest[..close_rel];
    if inner.is_empty() || inner.contains(char::is_whitespace) || inner.contains('<') {
        return None;
    }
    if let Some(colon) = inner.find(':') {
        let scheme = &inner[..colon];
        if scheme.len() >= 2
            && scheme.chars().next()?.is_ascii_alphabetic()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        {
            let escaped = escape_cm_html(inner);
            return Some((
                format!("<a href=\"{escaped}\">{escaped}</a>"),
                start + 1 + close_rel + 1,
            ));
        }
    }
    // Email autolink
    if let Some((local, domain)) = inner.split_once('@')
        && !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c))
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        let escaped = escape_cm_html(inner);
        return Some((
            format!("<a href=\"mailto:{escaped}\">{escaped}</a>"),
            start + 1 + close_rel + 1,
        ));
    }
    None
}

fn try_parse_inline_link(s: &str, start: usize) -> Option<(&str, &str, Option<&str>, usize)> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = start + 1;
    let mut depth = 1usize;
    while i < len {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                let run = s[i..].bytes().take_while(|&c| c == b'`').count();
                if let Some((_, next)) = find_closing_ticks(s, i + run, run) {
                    i = next;
                } else {
                    i += run;
                }
            }
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 || i >= len {
        return None;
    }
    let label = &s[start + 1..i];
    let after_bracket = i + 1;
    if after_bracket >= len || bytes[after_bracket] != b'(' {
        return None;
    }
    let mut j = after_bracket + 1;
    let mut paren_depth = 1usize;
    let mut in_quotes = false;
    while j < len {
        let c = bytes[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == b'"' {
            in_quotes = !in_quotes;
            j += 1;
            continue;
        }
        if !in_quotes {
            if c == b'(' {
                paren_depth += 1;
            } else if c == b')' {
                paren_depth -= 1;
                if paren_depth == 0 {
                    break;
                }
            }
        }
        j += 1;
    }
    if paren_depth != 0 || j >= len {
        return None;
    }
    let inside_parens = s[after_bracket + 1..j].trim();
    let (url, title) = parse_link_dest_and_title(inside_parens)?;
    Some((label, url, title, j + 1))
}

fn parse_link_dest_and_title(inside: &str) -> Option<(&str, Option<&str>)> {
    if inside.is_empty() {
        return Some(("", None));
    }
    if let Some(ws_idx) = inside.find(char::is_whitespace) {
        let url = &inside[..ws_idx];
        let rest = inside[ws_idx..].trim();
        if rest.is_empty() {
            return Some((url, None));
        }
        if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
            let title = &rest[1..rest.len() - 1];
            return Some((url, Some(title)));
        }
        None
    } else {
        Some((inside, None))
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

fn prev_char(s: &str, idx: usize) -> Option<char> {
    s[..idx].chars().next_back()
}

fn next_char(s: &str, idx: usize) -> Option<char> {
    s[idx..].chars().next()
}

fn try_parse_emphasis(s: &str, start: usize) -> Option<(&'static str, &str, usize)> {
    let bytes = s.as_bytes();
    let delim = bytes[start];
    let delim_char = delim as char;
    let run_len = s[start..].bytes().take_while(|&b| b == delim).count();

    let before = prev_char(s, start);
    // Simplified 6.2: intraword '_' cannot open emphasis
    if delim == b'_' && before.is_some_and(is_word_char) {
        return None;
    }

    // Try strong (** or __) first when at least 2 delimiters are present, then em (* or _)
    let candidates: &[usize] = if run_len >= 2 { &[2, 1] } else { &[1] };

    for &dlen in candidates {
        let content_start = start + dlen;
        let Some(first_inside) = next_char(s, content_start) else {
            continue;
        };
        if first_inside.is_whitespace() || first_inside == delim_char {
            continue;
        }

        if let Some(close_pos) = find_closing_emphasis(s, content_start, delim, dlen) {
            let tag = if dlen == 2 { "strong" } else { "em" };
            return Some((tag, &s[content_start..close_pos], close_pos + dlen));
        }
    }

    None
}

fn find_closing_emphasis(s: &str, content_start: usize, delim: u8, dlen: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = content_start;

    while i < len {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == b'`' {
            let run = s[i..].bytes().take_while(|&c| c == b'`').count();
            if let Some((_, next)) = find_closing_ticks(s, i + run, run) {
                i = next;
            } else {
                i += run;
            }
            continue;
        }
        if b == delim {
            let run = s[i..].bytes().take_while(|&c| c == delim).count();
            if run >= dlen {
                let prev = prev_char(s, i);
                let after = next_char(s, i + dlen);
                let valid_close = prev.is_some_and(|c| !c.is_whitespace())
                    && (delim != b'_' || !after.is_some_and(is_word_char));
                if valid_close {
                    return Some(i);
                }
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}
