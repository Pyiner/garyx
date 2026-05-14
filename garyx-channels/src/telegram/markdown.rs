pub(super) const MARKDOWN_V2_PARSE_MODE: &str = "MarkdownV2";

const MARKDOWN_V2_SPECIALS: &[char] = &[
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
];

pub(super) fn render_markdown_v2(text: &str) -> String {
    render_markdown_segments(text)
}

pub(super) fn is_markdown_parse_error(message: &str) -> bool {
    let lowered = message.to_lowercase();
    lowered.contains("can't parse entities")
        || lowered.contains("can't parse message text")
        || (lowered.contains("parse") && lowered.contains("entities"))
}

fn render_inline_markdown_v2(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut pos = 0;

    while pos < text.len() {
        let rest = &text[pos..];

        if let Some((inner, next_pos)) = parse_inline_delimited(rest, "**", "**") {
            output.push('*');
            output.push_str(&render_inline_markdown_v2(inner));
            output.push('*');
            pos += next_pos;
            continue;
        }

        if let Some((inner, next_pos)) = parse_inline_delimited(rest, "__", "__") {
            output.push('*');
            output.push_str(&render_inline_markdown_v2(inner));
            output.push('*');
            pos += next_pos;
            continue;
        }

        if let Some((label, url, next_pos)) = parse_link(rest) {
            output.push('[');
            output.push_str(&escape_markdown_v2_text(label));
            output.push_str("](");
            output.push_str(&escape_markdown_v2_url(url));
            output.push(')');
            pos += next_pos;
            continue;
        }

        if let Some((code, next_pos)) = parse_inline_delimited(rest, "`", "`") {
            output.push('`');
            output.push_str(&escape_markdown_v2_code(code));
            output.push('`');
            pos += next_pos;
            continue;
        }

        if let Some((inner, next_pos)) = parse_inline_delimited(rest, "*", "*") {
            output.push('_');
            output.push_str(&render_inline_markdown_v2(inner));
            output.push('_');
            pos += next_pos;
            continue;
        }

        if let Some((inner, next_pos)) = parse_inline_delimited(rest, "_", "_") {
            output.push('_');
            output.push_str(&render_inline_markdown_v2(inner));
            output.push('_');
            pos += next_pos;
            continue;
        }

        let ch = rest.chars().next().expect("non-empty rest");
        push_escaped_text_char(&mut output, ch);
        pos += ch.len_utf8();
    }

    output
}

fn render_markdown_segments(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(fence_offset) = text[pos..].find("```") {
        let fence_start = pos + fence_offset;
        output.push_str(&render_inline_markdown_v2(&text[pos..fence_start]));

        let rest = &text[fence_start..];
        let Some((rendered, consumed)) = parse_fenced_code_block(rest) else {
            output.push_str(&render_inline_markdown_v2(rest));
            return output;
        };
        output.push_str(&rendered);
        pos = fence_start + consumed;
    }

    output.push_str(&render_inline_markdown_v2(&text[pos..]));
    output
}

fn parse_fenced_code_block(text: &str) -> Option<(String, usize)> {
    let after_open = text.strip_prefix("```")?;
    let first_newline = after_open.find('\n')?;
    let info = after_open[..first_newline].trim();
    let code_start = "```".len() + first_newline + '\n'.len_utf8();
    let code_rest = &text[code_start..];
    let close_offset = code_rest.find("\n```")?;
    let code = &code_rest[..close_offset];
    let close_start = code_start + close_offset;
    let consumed = close_start + "\n```".len();

    let mut output = String::new();
    output.push_str("```");
    if let Some(language) = markdown_v2_code_language(info) {
        output.push_str(language);
    }
    output.push('\n');
    output.push_str(&escape_markdown_v2_code(code));
    output.push_str("\n```");

    Some((output, consumed))
}

fn markdown_v2_code_language(info: &str) -> Option<&str> {
    let language = info.split_whitespace().next().unwrap_or_default();
    if language.is_empty() {
        return None;
    }
    language
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+'))
        .then_some(language)
}

fn parse_inline_delimited<'a>(
    text: &'a str,
    opener: &str,
    closer: &str,
) -> Option<(&'a str, usize)> {
    let (inner, next_pos) = parse_delimited(text, opener, closer)?;
    if inner.contains('\n') {
        return None;
    }
    Some((inner, next_pos))
}

fn parse_delimited<'a>(text: &'a str, opener: &str, closer: &str) -> Option<(&'a str, usize)> {
    let inner_start = text.strip_prefix(opener)?;
    let close_at = inner_start.find(closer)?;
    if close_at == 0 {
        return None;
    }
    let inner = &inner_start[..close_at];
    let next_pos = opener.len() + close_at + closer.len();
    Some((inner, next_pos))
}

fn parse_link(text: &str) -> Option<(&str, &str, usize)> {
    let rest = text.strip_prefix('[')?;
    let label_end = rest.find("](")?;
    let label = &rest[..label_end];
    if label.is_empty() || label.contains(['\n', '\r', '[', ']']) {
        return None;
    }
    let url_start = label_end + 2;
    let url_rest = &rest[url_start..];
    let url_end = url_rest.find(')')?;
    let url = &url_rest[..url_end];
    if url.trim().is_empty() || url.contains(['\n', '\r']) {
        return None;
    }
    let next_pos = 1 + url_start + url_end + 1;
    Some((label, url, next_pos))
}

fn escape_markdown_v2_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        push_escaped_text_char(&mut output, ch);
    }
    output
}

fn push_escaped_text_char(output: &mut String, ch: char) {
    if ch == '\\' || MARKDOWN_V2_SPECIALS.contains(&ch) {
        output.push('\\');
    }
    output.push(ch);
}

fn escape_markdown_v2_code(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '`' || ch == '\\' {
            output.push('\\');
        }
        output.push(ch);
    }
    output
}

fn escape_markdown_v2_url(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == ')' || ch == '\\' {
            output.push('\\');
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::render_markdown_v2;

    #[test]
    fn literal_bracket_does_not_consume_later_link() {
        let rendered = render_markdown_v2(
            "Special characters:\n\n\
             _ * [ ] ( ) ~ ` > # + - = | { } . !\n\n\
             Mixed inline markdown:\n\n\
             This is **bold**, this is _italic_, this is `code`, link: [Telegram](https://telegram.org).",
        );

        assert!(rendered.contains(r"\_ \* \[ \] \( \) \~ \` \> \# \+ \- \= \| \{ \} \. \!"));
        assert!(rendered.contains(
            "This is *bold*, this is _italic_, this is `code`, link: [Telegram](https://telegram.org)\\."
        ));
        assert!(!rendered.contains(r"This is \*\*bold\*\*"));
    }
}
