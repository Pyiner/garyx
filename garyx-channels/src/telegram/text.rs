/// Strip `@bot_username` mentions from incoming text.
pub(super) fn strip_mention(text: &str, bot_username: &str) -> String {
    if bot_username.is_empty() {
        return text.to_string();
    }

    let mention = format!("@{bot_username}");
    let lower = text.to_lowercase();
    let mention_lower = mention.to_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(found) = lower[pos..].find(&mention_lower) {
        result.push_str(&text[pos..pos + found]);
        pos += found + mention.len();
    }

    result.push_str(&text[pos..]);
    result.trim().to_string()
}

/// Return a UTF-8-safe preview string by byte budget.
pub(crate) fn safe_log_preview(text: &str, max_bytes: usize) -> &str {
    if text.is_empty() || max_bytes == 0 {
        return "";
    }
    let end = floor_char_boundary(text, text.len().min(max_bytes));
    &text[..end]
}

/// Split a long message into chunks that fit Telegram's max message length.
/// Tries to split on newline boundaries for readability.
/// Handles multi-byte UTF-8 correctly by only splitting at char boundaries.
pub(super) fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find the largest char-boundary byte index <= max_len
        let safe_max = floor_char_boundary(remaining, max_len);

        // Try to find a good split point (newline) within the safe range
        let search_range = &remaining[..safe_max];
        let split_at = search_range
            .rfind('\n')
            .filter(|&pos| pos > safe_max / 4) // Don't split too early
            .unwrap_or(safe_max);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

/// Find the largest byte index `<= index` that is a char boundary in `s`.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
