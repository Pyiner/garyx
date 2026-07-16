use std::sync::Arc;

use async_trait::async_trait;
use garyx_router::recent_threads::{
    RecentThreadFilter, RecentThreadListEntry, RecentThreadPage, RecentThreadPageReader,
};

use crate::garyx_db::{GaryxDbService, RecentThreadTaskFilter};

pub(crate) struct SqlRecentThreadPageReader {
    garyx_db: Arc<GaryxDbService>,
}

impl SqlRecentThreadPageReader {
    pub(crate) fn new(garyx_db: Arc<GaryxDbService>) -> Self {
        Self { garyx_db }
    }
}

#[async_trait]
impl RecentThreadPageReader for SqlRecentThreadPageReader {
    async fn page(
        &self,
        filter: RecentThreadFilter,
        limit: usize,
        offset: usize,
    ) -> Result<RecentThreadPage, String> {
        let filter = match filter {
            RecentThreadFilter::Include => RecentThreadTaskFilter::Include,
            RecentThreadFilter::Exclude => RecentThreadTaskFilter::Exclude,
            RecentThreadFilter::Only => RecentThreadTaskFilter::Only,
        };
        self.garyx_db
            .run_blocking(move |db| db.list_recent_threads_page(filter, limit, offset))
            .await
            .map(|page| RecentThreadPage {
                entries: page
                    .records
                    .into_iter()
                    .map(|record| RecentThreadListEntry {
                        thread_id: record.thread_id,
                        title: record.title,
                        last_message_preview: record.last_message_preview,
                        last_active_at: record.last_active_at,
                    })
                    .collect(),
                total: page.total,
                offset: page.offset,
                has_more: page.has_more,
            })
            .map_err(|error| error.to_string())
    }

    async fn contains_selectable_thread(&self, thread_id: &str) -> Result<bool, String> {
        let thread_id = thread_id.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.contains_selectable_recent_thread(&thread_id))
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::app_bootstrap::AppStateBuilder;
    use crate::garyx_db::RecentThreadDraft;
    use garyx_models::config::GaryxConfig;

    fn draft(thread_id: &str, thread_type: &str, timestamp: &str) -> RecentThreadDraft {
        RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: format!("Title for {thread_id}"),
            workspace_dir: None,
            thread_type: thread_type.to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: "preview".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some(timestamp.to_owned()),
            last_active_at: timestamp.to_owned(),
        }
    }

    #[tokio::test]
    async fn sql_reader_maps_filtered_pages_and_selectability() {
        let db = Arc::new(GaryxDbService::memory().expect("memory db"));
        db.upsert_recent_thread(draft("thread::reader-task", "task", "2026-07-11T02:00:00Z"))
            .expect("task row");
        db.upsert_recent_thread(draft(
            "thread::reader-chat-older",
            "chat",
            "2026-07-11T00:00:00Z",
        ))
        .expect("older chat row");
        db.upsert_recent_thread(draft("thread::reader-chat", "chat", "2026-07-11T01:00:00Z"))
            .expect("chat row");
        let reader = SqlRecentThreadPageReader::new(db.clone());

        let page = reader
            .page(RecentThreadFilter::Exclude, 10, 0)
            .await
            .expect("page");
        assert_eq!(page.total, 2);
        assert_eq!(page.entries[0].thread_id, "thread::reader-chat");
        assert_eq!(page.entries[1].thread_id, "thread::reader-chat-older");
        assert_eq!(page.entries[0].last_message_preview, "preview");
        let api_chats = db
            .list_recent_threads_keyset_page(RecentThreadTaskFilter::Exclude, 10, None)
            .expect("api recent chats page");
        assert_eq!(page.total, api_chats.total);
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.thread_id.as_str())
                .collect::<Vec<_>>(),
            api_chats
                .records
                .iter()
                .map(|record| record.thread_id.as_str())
                .collect::<Vec<_>>(),
            "bot /threads must remain membership/order-equivalent to /api/recent-threads?tasks=exclude"
        );
        assert!(
            reader
                .contains_selectable_thread("thread::reader-chat")
                .await
                .expect("chat lookup")
        );
        assert!(
            !reader
                .contains_selectable_thread("thread::reader-task")
                .await
                .expect("task lookup")
        );
    }

    #[tokio::test]
    async fn app_state_builder_injects_the_sql_reader() {
        let state = AppStateBuilder::new(GaryxConfig::default()).build();
        let reader = state
            .threads
            .router
            .lock()
            .await
            .recent_thread_page_reader()
            .expect("recent reader");
        let page = reader
            .page(RecentThreadFilter::Exclude, 10, 0)
            .await
            .expect("empty page");
        assert_eq!(page.total, 0);
        assert!(page.entries.is_empty());
    }
}
