use super::*;

pub struct ThreadHistoryRepository {
    thread_store: Arc<dyn ThreadStore>,
    transcript_store: Arc<ThreadTranscriptStore>,
}

impl ThreadHistoryRepository {
    pub fn new(
        thread_store: Arc<dyn ThreadStore>,
        transcript_store: Arc<ThreadTranscriptStore>,
    ) -> Self {
        Self {
            thread_store,
            transcript_store,
        }
    }

    pub fn transcript_store(&self) -> Arc<ThreadTranscriptStore> {
        self.transcript_store.clone()
    }

    pub async fn thread_snapshot(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        self.thread_snapshot_page(thread_id, limit, None).await
    }

    pub async fn thread_snapshot_page(
        &self,
        thread_id: &str,
        limit: usize,
        before_index: Option<usize>,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .map_err(|error| ThreadHistoryError::Storage(error.to_string()))?
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_limit = limit.max(1);

        if let Some(before_index) = before_index {
            let (committed_messages, total_committed_messages, committed_start_index) = self
                .load_committed_messages_before_index(
                    thread_id,
                    &thread_data,
                    before_index,
                    bounded_limit,
                )
                .await?;
            return Ok(ThreadHistorySnapshot {
                thread_id: thread_id.to_owned(),
                thread_data,
                committed_messages,
                total_committed_messages,
                committed_start_index,
            });
        }

        let (committed_messages, total_committed_messages) = self
            .load_committed_messages(thread_id, &thread_data, bounded_limit)
            .await?;
        let committed_start_index =
            total_committed_messages.saturating_sub(committed_messages.len());

        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
        })
    }

    /// Forward delta snapshot: committed messages strictly after `after_index`.
    pub async fn thread_snapshot_after_index(
        &self,
        thread_id: &str,
        after_index: usize,
        limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .map_err(|error| ThreadHistoryError::Storage(error.to_string()))?
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_limit = limit.max(1);
        let (committed_messages, total_committed_messages, committed_start_index) = self
            .load_committed_messages_after_index(
                thread_id,
                &thread_data,
                after_index,
                bounded_limit,
            )
            .await?;
        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
        })
    }

    pub async fn thread_snapshot_user_query_page(
        &self,
        thread_id: &str,
        fallback_message_limit: usize,
        before_index: Option<usize>,
        user_query_limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .map_err(|error| ThreadHistoryError::Storage(error.to_string()))?
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_fallback_limit = fallback_message_limit.max(1);
        let bounded_user_query_limit = user_query_limit.max(1);

        if let Some(before_index) = before_index {
            let (committed_messages, total_committed_messages, committed_start_index) = self
                .load_committed_messages_before_user_queries(
                    thread_id,
                    &thread_data,
                    Some(before_index),
                    bounded_user_query_limit,
                    bounded_fallback_limit,
                )
                .await?;
            return Ok(ThreadHistorySnapshot {
                thread_id: thread_id.to_owned(),
                thread_data,
                committed_messages,
                total_committed_messages,
                committed_start_index,
            });
        }

        let (committed_messages, total_committed_messages, committed_start_index) = self
            .load_committed_messages_before_user_queries(
                thread_id,
                &thread_data,
                None,
                bounded_user_query_limit,
                bounded_fallback_limit,
            )
            .await?;

        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
        })
    }

    /// Newest `limit` provider-session content messages from the committed
    /// transcript, in transcript order (control records skipped). Returns
    /// an empty vector when the thread has no transcript yet — callers
    /// holding a legacy thread-record `messages` snapshot fall back to it
    /// until Batch 2's import backfills those transcripts (#TASK-1864).
    pub async fn provider_session_tail(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        if !self.transcript_store.exists(thread_id).await {
            return Ok(Vec::new());
        }
        self.transcript_store
            .provider_session_tail(thread_id, limit)
            .await
    }

    pub async fn find_latest_for_run(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() {
            return Ok(Vec::new());
        }
        self.thread_store
            .get(thread_id)
            .await
            .map_err(|error| ThreadHistoryError::Storage(error.to_string()))?
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_for_run(thread_id, trimmed_run_id)
                .await;
        }

        Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()))
    }

    pub async fn latest_message_text(
        &self,
        thread_id: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let snapshot = self.thread_snapshot(thread_id, 1).await?;
        let combined = snapshot.combined_messages();
        Ok(combined
            .last()
            .and_then(message_text)
            .map(|value| value.to_owned()))
    }

    pub async fn latest_message_text_for_role(
        &self,
        thread_id: &str,
        role: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let trimmed_role = role.trim();
        if trimmed_role.is_empty() {
            return Ok(None);
        }
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .map_err(|error| ThreadHistoryError::Storage(error.to_string()))?
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_text_for_role(thread_id, trimmed_role)
                .await;
        }

        if history_message_count(&thread_data) > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        Ok(None)
    }

    pub async fn delete_thread_history(&self, thread_id: &str) -> Result<(), ThreadHistoryError> {
        self.transcript_store.delete(thread_id).await
    }

    async fn load_committed_messages(
        &self,
        thread_id: &str,
        thread_data: &Value,
        limit: usize,
    ) -> Result<(Vec<Value>, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            let total = self.transcript_store.message_count(thread_id).await?;
            if limit == 0 {
                return Ok((Vec::new(), total));
            }
            return Ok((self.transcript_store.tail(thread_id, limit).await?, total));
        }

        Ok((Vec::new(), 0))
    }

    async fn load_committed_messages_before_index(
        &self,
        thread_id: &str,
        thread_data: &Value,
        before_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            if limit == 0 {
                let total = self.transcript_store.message_count(thread_id).await?;
                return Ok((Vec::new(), total, before_index.min(total)));
            }
            return self
                .transcript_store
                .page_before_index(thread_id, Some(before_index), limit)
                .await;
        }

        Ok((Vec::new(), 0, 0))
    }

    async fn load_committed_messages_after_index(
        &self,
        thread_id: &str,
        thread_data: &Value,
        after_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }
        if has_transcript {
            let total = self.transcript_store.message_count(thread_id).await?;
            if limit == 0 {
                return Ok((Vec::new(), total, after_index.saturating_add(1).min(total)));
            }
            return self
                .transcript_store
                .page_after_index(thread_id, after_index, limit)
                .await;
        }
        Ok((Vec::new(), 0, 0))
    }

    async fn load_committed_messages_before_user_queries(
        &self,
        thread_id: &str,
        thread_data: &Value,
        before_index: Option<usize>,
        user_query_limit: usize,
        fallback_message_limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            return self
                .transcript_store
                .page_before_user_queries(
                    thread_id,
                    before_index,
                    user_query_limit,
                    fallback_message_limit,
                )
                .await;
        }

        Ok((Vec::new(), 0, 0))
    }
}
