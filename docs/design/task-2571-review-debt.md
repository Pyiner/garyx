# Task 2571 Review Debt

Adjacent pre-existing issues found while reviewing, recorded per the review
scope boundary. They are not FAIL/BLOCKER drivers for the round that filed
them.

## From #TASK-2583 adversarial review of #TASK-2580 (round 1, 2026-07-22)

1. **`recent_threads` projection lacks workspace-membership columns.**
   `/api/thread-summaries` projects `root_workspace_path` and
   `workspace_origin`, but `RecentThreadRecord`
   (`garyx-gateway/src/garyx_db/recent.rs`) and the `/api/recent-threads`
   payload do not. Any client display rule keyed on workspace origin can
   therefore not be applied uniformly to Recent-route rows. The D3 subtitle
   blocker reported in the round-1 review is the changed-path symptom; this
   entry tracks the underlying projection gap that a root-cause fix likely
   needs (add the columns and their write-path derivation, per the
   projection contract).

2. **`thread_preview_user_first_v1` hard-fails boot on legacy orphans.**
   The cutover aborts startup (`GaryxDbError::Configuration`) if a
   `thread_meta` row has no canonical `thread_records` row, or if a record
   body fails to decode / has an invalid thread key. The 2026-07-22
   rehearsal on an isolated copy of a real production database found 0 such
   rows across 3,447 meta rows, so this is not currently reachable locally,
   but pre-contract bare deletes on aged installations could turn the next
   boot into a crash loop. Consider a tolerate-and-log path or a versioned
   repair instead of a hard abort.

3. **Cross-route freshness mixes two timestamp fields.**
   `GaryxThreadSummaryFreshness` compares Recent `last_active_at` with
   Summary `updated_at` as one timestamp domain. Today they are equal by
   construction (`recent_thread_projection.rs` derives `last_active_at`
   from record `updated_at`), but the coupling is implicit; if
   `last_active_at` semantics ever diverge from record `updated_at`,
   cross-route freshness decisions silently change. Worth an importable
   contract note or a shared derivation.
