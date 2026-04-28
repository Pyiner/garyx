//! Session label validation and management.
//!
//! Provides:
//! - Label validation (max length, allowed characters)
//! - Case-insensitive label matching
//! - Label normalization for search

use once_cell::sync::Lazy;
use regex::Regex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const MAX_LABEL_LENGTH: usize = 64;
pub const MIN_LABEL_LENGTH: usize = 1;

/// Label pattern: alphanumeric, hyphens, underscores, spaces.
/// Must not start or end with whitespace.
static LABEL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_\- ]*[a-zA-Z0-9]$|^[a-zA-Z0-9]$").unwrap());

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of label validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelValidationResult {
    pub ok: bool,
    pub label: Option<String>,
    pub error: Option<String>,
}

impl LabelValidationResult {
    fn success(label: String) -> Self {
        Self {
            ok: true,
            label: Some(label),
            error: None,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            label: None,
            error: Some(error.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate and normalize a session label.
pub fn validate_session_label(raw_label: &str) -> LabelValidationResult {
    let label = raw_label.trim();

    if label.is_empty() {
        return LabelValidationResult::failure("Label cannot be empty");
    }

    if label.len() < MIN_LABEL_LENGTH {
        return LabelValidationResult::failure(format!(
            "Label must be at least {MIN_LABEL_LENGTH} character(s)"
        ));
    }

    if label.len() > MAX_LABEL_LENGTH {
        return LabelValidationResult::failure(format!(
            "Label cannot exceed {MAX_LABEL_LENGTH} characters"
        ));
    }

    if !LABEL_PATTERN.is_match(label) {
        return LabelValidationResult::failure(
            "Label must contain only letters, numbers, hyphens, underscores, and spaces",
        );
    }

    LabelValidationResult::success(label.to_owned())
}

/// Normalize a label for case-insensitive search.
pub fn normalize_label_for_search(label: &str) -> String {
    label.trim().to_lowercase()
}

/// Check if two labels match (case-insensitive).
pub fn labels_match(label1: &str, label2: &str) -> bool {
    normalize_label_for_search(label1) == normalize_label_for_search(label2)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests;
