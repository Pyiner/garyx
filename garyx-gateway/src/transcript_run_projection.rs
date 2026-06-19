use std::sync::Arc;

use garyx_router::ThreadTranscriptStore;

pub(crate) async fn active_run_id_from_transcript_store(
    transcript_store: &Arc<ThreadTranscriptStore>,
    thread_id: &str,
) -> Option<String> {
    let state = transcript_store.run_state(thread_id).await.ok()?;
    if state.busy {
        state.active_run_id
    } else {
        None
    }
}
