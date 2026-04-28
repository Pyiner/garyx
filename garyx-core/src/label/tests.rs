use super::*;

#[test]
fn test_valid_label() {
    let r = validate_session_label("My Session");
    assert!(r.ok);
    assert_eq!(r.label.as_deref(), Some("My Session"));
    assert!(r.error.is_none());
}

#[test]
fn test_single_char_label() {
    let r = validate_session_label("A");
    assert!(r.ok);
    assert_eq!(r.label.as_deref(), Some("A"));
}

#[test]
fn test_empty_label() {
    let r = validate_session_label("");
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("empty"));
}

#[test]
fn test_whitespace_only_label() {
    let r = validate_session_label("   ");
    assert!(!r.ok);
}

#[test]
fn test_label_trimmed() {
    let r = validate_session_label("  hello  ");
    assert!(r.ok);
    assert_eq!(r.label.as_deref(), Some("hello"));
}

#[test]
fn test_label_too_long() {
    let long = "a".repeat(MAX_LABEL_LENGTH + 1);
    let r = validate_session_label(&long);
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("exceed"));
}

#[test]
fn test_label_max_length_ok() {
    let label = "a".repeat(MAX_LABEL_LENGTH);
    let r = validate_session_label(&label);
    assert!(r.ok);
}

#[test]
fn test_label_with_special_chars_rejected() {
    let r = validate_session_label("hello!");
    assert!(!r.ok);
}

#[test]
fn test_label_with_allowed_chars() {
    let r = validate_session_label("my-session_v2");
    assert!(r.ok);
}

#[test]
fn test_label_starting_with_hyphen_rejected() {
    let r = validate_session_label("-hello");
    assert!(!r.ok);
}

#[test]
fn test_normalize_for_search() {
    assert_eq!(normalize_label_for_search("  Hello World  "), "hello world");
}

#[test]
fn test_labels_match_case_insensitive() {
    assert!(labels_match("Hello", "hello"));
    assert!(labels_match("  Test  ", "test"));
    assert!(!labels_match("abc", "def"));
}

#[test]
fn test_labels_match_with_whitespace() {
    assert!(labels_match("  foo  ", "foo"));
}
