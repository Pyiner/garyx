use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MarkdownImageRef {
    pub(super) path: PathBuf,
    pub(super) alt: Option<String>,
    source_range: Range<usize>,
}

fn supported_markdown_image_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn markdown_image_target_path(raw_target: &str) -> Option<PathBuf> {
    let mut target = raw_target.trim();
    if target.is_empty() {
        return None;
    }

    if let Some(stripped) = target
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    {
        target = stripped.trim();
    } else if let Some(index) = target.find(char::is_whitespace) {
        target = target[..index].trim();
    }

    let target = target.trim_matches(|value| value == '"' || value == '\'');
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("data:")
    {
        return None;
    }

    let path = target
        .strip_prefix("file://")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(target));
    path.is_absolute()
        .then_some(path)
        .filter(|path| supported_markdown_image_extension(path))
        .filter(|path| path.is_file())
}

fn scan_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut offset = 0;

    while let Some(relative_start) = text[offset..].find("![") {
        let start = offset + relative_start;
        let alt_start = start + 2;
        let Some(alt_end_relative) = text[alt_start..].find("](") else {
            offset = alt_start;
            continue;
        };
        let alt_end = alt_start + alt_end_relative;
        let target_start = alt_end + 2;
        let Some(target_end_relative) = text[target_start..].find(')') else {
            offset = target_start;
            continue;
        };
        let target_end = target_start + target_end_relative;
        let alt = text[alt_start..alt_end].trim();
        let target = &text[target_start..target_end];

        if let Some(path) = markdown_image_target_path(target) {
            refs.push(MarkdownImageRef {
                path,
                alt: (!alt.is_empty()).then(|| alt.to_owned()),
                source_range: start..target_end + 1,
            });
        }

        offset = target_end + 1;
    }
    refs
}

pub(super) fn extract_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for image_ref in scan_markdown_image_refs(text) {
        let key = image_ref.path.to_string_lossy().to_string();
        if seen.insert(key) {
            refs.push(image_ref);
        }
    }
    refs
}

pub(super) fn strip_deliverable_markdown_images(text: &str) -> String {
    let refs = scan_markdown_image_refs(text);
    if refs.is_empty() {
        return text.to_owned();
    }

    let mut stripped = String::with_capacity(text.len());
    let mut cursor = 0;
    for image_ref in refs {
        if image_ref.source_range.start > cursor {
            stripped.push_str(&text[cursor..image_ref.source_range.start]);
        }
        cursor = image_ref.source_range.end;
    }
    if cursor < text.len() {
        stripped.push_str(&text[cursor..]);
    }
    while stripped.contains("\n\n\n") {
        stripped = stripped.replace("\n\n\n", "\n\n");
    }
    stripped.trim().to_owned()
}
