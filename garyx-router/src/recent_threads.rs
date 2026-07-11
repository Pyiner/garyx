use std::collections::{BTreeMap, HashMap, VecDeque};

use async_trait::async_trait;
use garyx_models::thread_logs::is_canonical_thread_id;

pub const RECENT_THREADS_PAGE_SIZE: usize = 10;
pub const RECENT_THREAD_SNAPSHOT_ENTRY_LIMIT: usize = 200;
pub const RECENT_THREAD_CONTEXT_LIMIT: usize = 512;
pub const RECENT_THREAD_GLOBAL_ENTRY_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecentThreadFilter {
    Include,
    #[default]
    Exclude,
    Only,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentThreadListEntry {
    pub thread_id: String,
    pub title: String,
    pub last_message_preview: String,
    pub last_active_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentThreadPage {
    pub entries: Vec<RecentThreadListEntry>,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[async_trait]
pub trait RecentThreadPageReader: Send + Sync {
    async fn page(
        &self,
        filter: RecentThreadFilter,
        limit: usize,
        offset: usize,
    ) -> Result<RecentThreadPage, String>;

    async fn contains_selectable_thread(&self, thread_id: &str) -> Result<bool, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentPageRequest {
    First,
    Explicit(usize),
    Next,
    Prev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecentBindRequest {
    SnapshotIndex(usize),
    DirectThreadId(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecentCommandParseError {
    #[error("Usage: /threads [page|next|prev]")]
    ThreadsUsage,
    #[error("Usage: /bindthread <number from /threads>")]
    BindUsage,
}

pub fn parse_recent_page_request(text: &str) -> Result<RecentPageRequest, RecentCommandParseError> {
    let (command, arguments) = command_and_arguments(text);
    if command.as_deref() != Some("threads") || arguments.len() > 1 {
        return Err(RecentCommandParseError::ThreadsUsage);
    }
    let Some(argument) = arguments.first() else {
        return Ok(RecentPageRequest::First);
    };
    match argument.to_ascii_lowercase().as_str() {
        "next" => Ok(RecentPageRequest::Next),
        "prev" => Ok(RecentPageRequest::Prev),
        _ => argument
            .parse::<usize>()
            .ok()
            .filter(|page| *page > 0)
            .map(RecentPageRequest::Explicit)
            .ok_or(RecentCommandParseError::ThreadsUsage),
    }
}

pub fn parse_recent_bind_request(text: &str) -> Result<RecentBindRequest, RecentCommandParseError> {
    let (command, arguments) = command_and_arguments(text);
    if command.as_deref() != Some("bindthread") || arguments.len() != 1 {
        return Err(RecentCommandParseError::BindUsage);
    }
    let argument = arguments[0];
    if let Ok(index) = argument.parse::<usize>()
        && index > 0
    {
        return Ok(RecentBindRequest::SnapshotIndex(index));
    }
    if is_canonical_thread_id(argument) {
        return Ok(RecentBindRequest::DirectThreadId(argument.to_owned()));
    }
    Err(RecentCommandParseError::BindUsage)
}

fn command_and_arguments(text: &str) -> (Option<String>, Vec<&str>) {
    let mut tokens = text.split_whitespace();
    let command = tokens.next().and_then(|token| {
        token
            .strip_prefix('/')
            .and_then(|token| token.split('@').next())
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| token.to_ascii_lowercase())
    });
    (command, tokens.collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentPageBoundaryNotice {
    First,
    Last,
}

impl RecentPageBoundaryNotice {
    pub fn message(self) -> &'static str {
        match self {
            Self::First => "Already on the first page.",
            Self::Last => "Already on the last page.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecentPageResolution {
    pub page: usize,
    pub total_pages: usize,
    pub notice: Option<RecentPageBoundaryNotice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Page {requested_page} is out of range ({total_pages} pages). Use /threads {total_pages}.")]
pub struct RecentPageOutOfRange {
    pub requested_page: usize,
    pub total_pages: usize,
}

pub fn recent_total_pages(total: usize, page_size: usize) -> usize {
    if total == 0 || page_size == 0 {
        return 1;
    }
    total.saturating_add(page_size - 1) / page_size
}

pub fn resolve_recent_page(
    request: RecentPageRequest,
    last_successful_page: Option<usize>,
    total: usize,
    page_size: usize,
) -> Result<RecentPageResolution, RecentPageOutOfRange> {
    let total_pages = recent_total_pages(total, page_size);
    let (page, notice) = match request {
        RecentPageRequest::First => (1, None),
        RecentPageRequest::Explicit(page) => {
            if page == 0 || page > total_pages {
                return Err(RecentPageOutOfRange {
                    requested_page: page,
                    total_pages,
                });
            }
            (page, None)
        }
        RecentPageRequest::Next => match last_successful_page {
            None => (1, None),
            Some(last_page) => {
                let current = last_page.clamp(1, total_pages);
                if current >= total_pages {
                    (total_pages, Some(RecentPageBoundaryNotice::Last))
                } else {
                    (current + 1, None)
                }
            }
        },
        RecentPageRequest::Prev => match last_successful_page {
            None => (1, None),
            Some(last_page) => {
                let current = last_page.clamp(1, total_pages);
                if current <= 1 {
                    (1, Some(RecentPageBoundaryNotice::First))
                } else {
                    (current - 1, None)
                }
            }
        },
    };
    Ok(RecentPageResolution {
        page,
        total_pages,
        notice,
    })
}

pub fn requested_recent_page(
    request: RecentPageRequest,
    last_successful_page: Option<usize>,
) -> usize {
    match request {
        RecentPageRequest::First => 1,
        RecentPageRequest::Explicit(page) => page,
        RecentPageRequest::Next => last_successful_page
            .map(|page| page.saturating_add(1))
            .unwrap_or(1),
        RecentPageRequest::Prev => last_successful_page
            .map(|page| page.saturating_sub(1).max(1))
            .unwrap_or(1),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentThreadDisplay {
    pub thread_id: String,
    pub title: String,
}

pub fn format_recent_thread_page(
    page: &RecentThreadPage,
    resolution: RecentPageResolution,
    current: Option<&CurrentThreadDisplay>,
) -> String {
    if page.total == 0 {
        return "No recent threads yet.\nUse /newthread to create one.".to_owned();
    }

    let mut lines = Vec::new();
    if let Some(notice) = resolution.notice {
        lines.push(notice.message().to_owned());
    }
    lines.push(format!(
        "Recent threads · page {}/{} ({} total)",
        resolution.page, resolution.total_pages, page.total
    ));
    let current_id = current.map(|current| current.thread_id.as_str());
    let mut current_on_page = false;
    for (index, entry) in page.entries.iter().enumerate() {
        let absolute_index = page.offset.saturating_add(index).saturating_add(1);
        let is_current = current_id == Some(entry.thread_id.as_str());
        current_on_page |= is_current;
        let marker = if is_current { " ⬅️" } else { "" };
        lines.push(format!(
            "{absolute_index}. {} [{}]{marker}",
            recent_thread_display_title(&entry.title, &entry.last_message_preview),
            short_thread_id(&entry.thread_id),
        ));
    }
    if let Some(current) = current
        && !current_on_page
    {
        lines.push(String::new());
        lines.push(format!(
            "Current: {}",
            normalize_and_truncate(&current.title, 64).unwrap_or_else(|| "New Thread".to_owned())
        ));
    }
    lines.push(String::new());
    let browse = if resolution.page < resolution.total_pages {
        format!("/threads {} → next", resolution.page + 1)
    } else if resolution.page > 1 {
        format!("/threads {} → previous", resolution.page - 1)
    } else {
        "/threads next → browse".to_owned()
    };
    lines.push(format!(
        "{browse} · /bindthread <n> → switch · /newthread → create"
    ));
    lines.join("\n")
}

pub(crate) fn recent_thread_display_title(title: &str, last_message_preview: &str) -> String {
    let normalized_title = normalize_text(title);
    let derived_placeholder = normalized_title
        .as_deref()
        .is_none_or(|title| title.eq_ignore_ascii_case("New Thread"));
    if derived_placeholder && let Some(preview) = normalize_and_truncate(last_message_preview, 48) {
        return preview;
    }
    normalized_title
        .and_then(|title| truncate_unicode(title.as_str(), 64))
        .unwrap_or_else(|| "New Thread".to_owned())
}

fn normalize_and_truncate(value: &str, max_chars: usize) -> Option<String> {
    normalize_text(value).and_then(|value| truncate_unicode(&value, max_chars))
}

fn normalize_text(value: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() || character.is_control() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(character);
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn truncate_unicode(value: &str, max_chars: usize) -> Option<String> {
    if max_chars == 0 {
        return None;
    }
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(max_chars).collect::<String>();
    if prefix.is_empty() {
        return None;
    }
    if characters.next().is_some() {
        Some(format!("{prefix}…"))
    } else {
        Some(prefix)
    }
}

fn short_thread_id(thread_id: &str) -> String {
    let value = thread_id
        .trim()
        .strip_prefix("thread::")
        .unwrap_or(thread_id.trim());
    let mut tail = value.chars().rev().take(8).collect::<Vec<_>>();
    tail.reverse();
    tail.into_iter().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentThreadSnapshotEntry {
    pub thread_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentBindSelection {
    pub thread_id: String,
    pub snapshot_title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Run /threads first, then /bindthread <n>.")]
pub struct RecentSnapshotUnavailable;

#[derive(Debug, Clone, Copy)]
struct RecentThreadBrowserLimits {
    entries_per_context: usize,
    contexts: usize,
    global_entries: usize,
}

impl Default for RecentThreadBrowserLimits {
    fn default() -> Self {
        Self {
            entries_per_context: RECENT_THREAD_SNAPSHOT_ENTRY_LIMIT,
            contexts: RECENT_THREAD_CONTEXT_LIMIT,
            global_entries: RECENT_THREAD_GLOBAL_ENTRY_LIMIT,
        }
    }
}

#[derive(Debug, Default)]
struct RecentThreadContextState {
    last_page: Option<usize>,
    total_pages: usize,
    entries: BTreeMap<usize, RecentThreadSnapshotEntry>,
    insertion_order: VecDeque<usize>,
    last_used: u64,
}

#[derive(Debug)]
pub struct RecentThreadBrowserState {
    contexts: HashMap<String, RecentThreadContextState>,
    access_clock: u64,
    limits: RecentThreadBrowserLimits,
}

impl Default for RecentThreadBrowserState {
    fn default() -> Self {
        Self {
            contexts: HashMap::new(),
            access_clock: 0,
            limits: RecentThreadBrowserLimits::default(),
        }
    }
}

impl RecentThreadBrowserState {
    #[cfg(test)]
    fn with_limits(entries_per_context: usize, contexts: usize, global_entries: usize) -> Self {
        Self {
            contexts: HashMap::new(),
            access_clock: 0,
            limits: RecentThreadBrowserLimits {
                entries_per_context: entries_per_context.max(1),
                contexts: contexts.max(1),
                global_entries: global_entries.max(1),
            },
        }
    }

    pub fn last_successful_page(&mut self, context_key: &str) -> Option<usize> {
        self.touch(context_key);
        self.contexts
            .get(context_key)
            .and_then(|context| context.last_page)
    }

    pub fn record_successful_page(
        &mut self,
        context_key: &str,
        resolution: RecentPageResolution,
        page: &RecentThreadPage,
    ) {
        let access = self.next_access();
        let per_context_limit = self.limits.entries_per_context;
        let context = self.contexts.entry(context_key.to_owned()).or_default();
        context.last_used = access;
        context.last_page = Some(resolution.page);
        context.total_pages = resolution.total_pages;
        for (index, entry) in page.entries.iter().enumerate() {
            let absolute_index = page.offset.saturating_add(index).saturating_add(1);
            context
                .insertion_order
                .retain(|candidate| *candidate != absolute_index);
            context.insertion_order.push_back(absolute_index);
            context.entries.insert(
                absolute_index,
                RecentThreadSnapshotEntry {
                    thread_id: entry.thread_id.clone(),
                    title: recent_thread_display_title(&entry.title, &entry.last_message_preview),
                },
            );
        }
        while context.entries.len() > per_context_limit {
            let Some(index) = context.insertion_order.pop_front() else {
                break;
            };
            context.entries.remove(&index);
        }
        self.enforce_context_limit();
        self.enforce_global_entry_limit();
    }

    pub fn resolve_bind_request(
        &self,
        context_key: &str,
        request: &RecentBindRequest,
    ) -> Result<RecentBindSelection, RecentSnapshotUnavailable> {
        if let RecentBindRequest::DirectThreadId(thread_id) = request {
            return Ok(RecentBindSelection {
                thread_id: thread_id.clone(),
                snapshot_title: None,
            });
        }
        let RecentBindRequest::SnapshotIndex(index) = request else {
            unreachable!();
        };
        let context = self
            .contexts
            .get(context_key)
            .ok_or(RecentSnapshotUnavailable)?;
        let entry = context
            .entries
            .get(index)
            .ok_or(RecentSnapshotUnavailable)?;
        Ok(RecentBindSelection {
            thread_id: entry.thread_id.clone(),
            snapshot_title: Some(entry.title.clone()),
        })
    }

    pub fn clear_context(&mut self, context_key: &str) {
        self.contexts.remove(context_key);
    }

    fn next_access(&mut self) -> u64 {
        self.access_clock = self.access_clock.saturating_add(1);
        self.access_clock
    }

    fn touch(&mut self, context_key: &str) {
        let access = self.next_access();
        if let Some(context) = self.contexts.get_mut(context_key) {
            context.last_used = access;
        }
    }

    fn enforce_context_limit(&mut self) {
        while self.contexts.len() > self.limits.contexts {
            let Some(key) = self.least_recent_context_key() else {
                break;
            };
            self.contexts.remove(&key);
        }
    }

    fn enforce_global_entry_limit(&mut self) {
        while self.total_entries() > self.limits.global_entries {
            let Some(key) = self.least_recent_context_key() else {
                break;
            };
            self.contexts.remove(&key);
        }
    }

    fn least_recent_context_key(&self) -> Option<String> {
        self.contexts
            .iter()
            .min_by(|(left_key, left), (right_key, right)| {
                left.last_used
                    .cmp(&right.last_used)
                    .then_with(|| left_key.cmp(right_key))
            })
            .map(|(key, _)| key.clone())
    }

    fn total_entries(&self) -> usize {
        self.contexts
            .values()
            .map(|context| context.entries.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn entry(thread_id: &str, title: &str, preview: &str) -> RecentThreadListEntry {
        RecentThreadListEntry {
            thread_id: thread_id.to_owned(),
            title: title.to_owned(),
            last_message_preview: preview.to_owned(),
            last_active_at: "2026-07-11T00:00:00Z".to_owned(),
        }
    }

    fn page(offset: usize, total: usize, entries: Vec<RecentThreadListEntry>) -> RecentThreadPage {
        RecentThreadPage {
            has_more: offset.saturating_add(entries.len()) < total,
            entries,
            total,
            offset,
        }
    }

    #[test]
    fn parser_accepts_pages_navigation_addressing_and_direct_ids() {
        assert_eq!(
            parse_recent_page_request("/threads").unwrap(),
            RecentPageRequest::First
        );
        assert_eq!(
            parse_recent_page_request(" /threads@sample_bot 12 ").unwrap(),
            RecentPageRequest::Explicit(12)
        );
        assert_eq!(
            parse_recent_page_request("/threads NEXT").unwrap(),
            RecentPageRequest::Next
        );
        assert_eq!(
            parse_recent_page_request("/threads prev").unwrap(),
            RecentPageRequest::Prev
        );
        assert_eq!(
            parse_recent_bind_request("/bindthread@sample_bot 21").unwrap(),
            RecentBindRequest::SnapshotIndex(21)
        );
        assert_eq!(
            parse_recent_bind_request("/bindthread thread::12345678-1234-1234-1234-123456789abc")
                .unwrap(),
            RecentBindRequest::DirectThreadId(
                "thread::12345678-1234-1234-1234-123456789abc".to_owned()
            )
        );
    }

    #[test]
    fn parser_rejects_zero_nonnumeric_overflow_and_extra_arguments() {
        for text in [
            "/threads 0",
            "/threads later",
            "/threads 184467440737095516161",
            "/threads 2 extra",
            "/other 2",
        ] {
            assert_eq!(
                parse_recent_page_request(text),
                Err(RecentCommandParseError::ThreadsUsage),
                "{text}"
            );
        }
        for text in [
            "/bindthread",
            "/bindthread 0",
            "/bindthread nope",
            "/bindthread 184467440737095516161",
            "/bindthread 2 extra",
        ] {
            assert_eq!(
                parse_recent_bind_request(text),
                Err(RecentCommandParseError::BindUsage),
                "{text}"
            );
        }
    }

    #[test]
    fn pager_handles_state_boundaries_and_explicit_out_of_range() {
        assert_eq!(
            resolve_recent_page(RecentPageRequest::Next, None, 32, 10).unwrap(),
            RecentPageResolution {
                page: 1,
                total_pages: 4,
                notice: None,
            }
        );
        assert_eq!(
            resolve_recent_page(RecentPageRequest::Next, Some(2), 32, 10).unwrap(),
            RecentPageResolution {
                page: 3,
                total_pages: 4,
                notice: None,
            }
        );
        assert_eq!(
            resolve_recent_page(RecentPageRequest::Next, Some(4), 32, 10).unwrap(),
            RecentPageResolution {
                page: 4,
                total_pages: 4,
                notice: Some(RecentPageBoundaryNotice::Last),
            }
        );
        assert_eq!(
            resolve_recent_page(RecentPageRequest::Prev, Some(1), 32, 10).unwrap(),
            RecentPageResolution {
                page: 1,
                total_pages: 4,
                notice: Some(RecentPageBoundaryNotice::First),
            }
        );
        let error = resolve_recent_page(RecentPageRequest::Explicit(7), Some(2), 32, 10)
            .expect_err("out of range");
        assert_eq!(
            error.to_string(),
            "Page 7 is out of range (4 pages). Use /threads 4."
        );
        assert_eq!(recent_total_pages(0, 10), 1);
    }

    #[test]
    fn formatter_renders_absolute_rows_short_ids_current_and_footer() {
        let rendered = format_recent_thread_page(
            &page(
                10,
                32,
                vec![
                    entry(
                        "thread::12345678-1234-1234-1234-aaaabbbbcccc",
                        "Fix\nlogin\tflow",
                        "",
                    ),
                    entry(
                        "thread::12345678-1234-1234-1234-ddddeeeeffff",
                        "Weekly report",
                        "",
                    ),
                ],
            ),
            RecentPageResolution {
                page: 2,
                total_pages: 4,
                notice: None,
            },
            Some(&CurrentThreadDisplay {
                thread_id: "thread::12345678-1234-1234-1234-aaaabbbbcccc".to_owned(),
                title: "Fix login flow".to_owned(),
            }),
        );
        assert_eq!(
            rendered,
            "Recent threads · page 2/4 (32 total)\n\
11. Fix login flow [bbbbcccc] ⬅️\n\
12. Weekly report [eeeeffff]\n\
\n\
/threads 3 → next · /bindthread <n> → switch · /newthread → create"
        );
    }

    #[test]
    fn formatter_uses_preview_unicode_truncation_and_off_page_current() {
        let long_preview = "界".repeat(50);
        let rendered = format_recent_thread_page(
            &page(
                0,
                2,
                vec![entry(
                    "thread::12345678-1234-1234-1234-123456789abc",
                    "New Thread",
                    &long_preview,
                )],
            ),
            RecentPageResolution {
                page: 1,
                total_pages: 1,
                notice: Some(RecentPageBoundaryNotice::Last),
            },
            Some(&CurrentThreadDisplay {
                thread_id: "thread::off-page".to_owned(),
                title: " Current\u{0000} thread ".to_owned(),
            }),
        );
        let preview = format!("{}…", "界".repeat(48));
        assert!(rendered.starts_with("Already on the last page.\nRecent threads · page 1/1"));
        assert!(rendered.contains(&format!("1. {preview} [56789abc]")));
        assert!(rendered.contains("\nCurrent: Current thread\n"));
        assert!(rendered.is_char_boundary(rendered.len()));
        assert_eq!(
            format_recent_thread_page(
                &page(0, 0, Vec::new()),
                RecentPageResolution {
                    page: 1,
                    total_pages: 1,
                    notice: None,
                },
                None,
            ),
            "No recent threads yet.\nUse /newthread to create one."
        );
    }

    #[test]
    fn snapshot_accumulates_pages_resolves_exact_rows_and_clears() {
        let mut state = RecentThreadBrowserState::default();
        state.record_successful_page(
            "telegram::main::1000000001",
            RecentPageResolution {
                page: 1,
                total_pages: 2,
                notice: None,
            },
            &page(
                0,
                12,
                vec![
                    entry("thread::one", "One", ""),
                    entry("thread::two", "Two", ""),
                ],
            ),
        );
        state.record_successful_page(
            "telegram::main::1000000001",
            RecentPageResolution {
                page: 2,
                total_pages: 2,
                notice: None,
            },
            &page(10, 12, vec![entry("thread::eleven", "Eleven", "")]),
        );
        assert_eq!(
            state.last_successful_page("telegram::main::1000000001"),
            Some(2)
        );
        assert_eq!(
            state
                .resolve_bind_request(
                    "telegram::main::1000000001",
                    &RecentBindRequest::SnapshotIndex(1),
                )
                .unwrap()
                .thread_id,
            "thread::one"
        );
        assert_eq!(
            state
                .resolve_bind_request(
                    "telegram::main::1000000001",
                    &RecentBindRequest::SnapshotIndex(11),
                )
                .unwrap()
                .thread_id,
            "thread::eleven"
        );
        assert!(
            state
                .resolve_bind_request(
                    "telegram::main::1000000001",
                    &RecentBindRequest::SnapshotIndex(9),
                )
                .is_err()
        );
        let direct = state
            .resolve_bind_request(
                "missing-context",
                &RecentBindRequest::DirectThreadId("thread::direct".to_owned()),
            )
            .unwrap();
        assert_eq!(direct.thread_id, "thread::direct");
        assert_eq!(direct.snapshot_title, None);

        state.clear_context("telegram::main::1000000001");
        assert_eq!(
            state.last_successful_page("telegram::main::1000000001"),
            None
        );
        assert!(
            state
                .resolve_bind_request(
                    "telegram::main::1000000001",
                    &RecentBindRequest::SnapshotIndex(1),
                )
                .is_err()
        );
    }

    #[test]
    fn snapshot_enforces_entry_context_and_global_limits_with_lru_eviction() {
        let mut state = RecentThreadBrowserState::with_limits(2, 2, 3);
        let resolution = RecentPageResolution {
            page: 1,
            total_pages: 1,
            notice: None,
        };
        state.record_successful_page(
            "context-a",
            resolution,
            &page(
                0,
                3,
                vec![
                    entry("thread::a1", "A1", ""),
                    entry("thread::a2", "A2", ""),
                    entry("thread::a3", "A3", ""),
                ],
            ),
        );
        assert!(
            state
                .resolve_bind_request("context-a", &RecentBindRequest::SnapshotIndex(1))
                .is_err(),
            "oldest entry must be evicted at the per-context limit"
        );
        state.record_successful_page(
            "context-b",
            resolution,
            &page(
                0,
                2,
                vec![entry("thread::b1", "B1", ""), entry("thread::b2", "B2", "")],
            ),
        );
        assert!(
            !state.contexts.contains_key("context-a"),
            "global budget must evict the least recently used whole context"
        );
        state.record_successful_page(
            "context-c",
            resolution,
            &page(0, 1, vec![entry("thread::c1", "C1", "")]),
        );
        state.last_successful_page("context-b");
        state.record_successful_page(
            "context-d",
            resolution,
            &page(0, 1, vec![entry("thread::d1", "D1", "")]),
        );
        assert!(state.contexts.contains_key("context-b"));
        assert!(state.contexts.contains_key("context-d"));
        assert!(!state.contexts.contains_key("context-c"));
        assert!(state.contexts.len() <= 2);
        assert!(state.total_entries() <= 3);
    }

    struct FakeReader {
        page: RecentThreadPage,
        selectable: bool,
    }

    #[async_trait]
    impl RecentThreadPageReader for FakeReader {
        async fn page(
            &self,
            _filter: RecentThreadFilter,
            _limit: usize,
            _offset: usize,
        ) -> Result<RecentThreadPage, String> {
            Ok(self.page.clone())
        }

        async fn contains_selectable_thread(&self, _thread_id: &str) -> Result<bool, String> {
            Ok(self.selectable)
        }
    }

    #[tokio::test]
    async fn fake_reader_provides_deterministic_headless_pages() {
        let reader: Arc<dyn RecentThreadPageReader> = Arc::new(FakeReader {
            page: page(0, 1, vec![entry("thread::fake", "Fake", "")]),
            selectable: true,
        });
        let page = reader
            .page(RecentThreadFilter::Exclude, 10, 0)
            .await
            .unwrap();
        assert_eq!(page.entries[0].thread_id, "thread::fake");
        assert!(
            reader
                .contains_selectable_thread("thread::fake")
                .await
                .unwrap()
        );
    }
}
