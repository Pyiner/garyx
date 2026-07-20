use std::fs;
use std::path::{Path, PathBuf};

fn production_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read gateway source directory") {
        let path = entry.expect("gateway source entry").path();
        if path.is_dir() {
            if path.file_name().and_then(|value| value.to_str()) == Some("tests") {
                continue;
            }
            production_rust_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs")
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name == "source_guard_tests.rs" || name.ends_with("tests.rs"))
        {
            files.push(path);
        }
    }
}

fn production_calls(source: &Path, needle: &str) -> Vec<String> {
    let mut files = Vec::new();
    production_rust_files(source, &mut files);
    let mut calls = Vec::new();
    for path in files {
        let body = fs::read_to_string(&path).expect("read gateway Rust source");
        let production = body.split("#[cfg(test)]\nmod ").next().unwrap_or(&body);
        for line in production.lines().filter(|line| line.contains(needle)) {
            calls.push(format!(
                "{}:{}",
                path.strip_prefix(source).unwrap().display(),
                line.trim()
            ));
        }
    }
    calls.sort();
    calls
}

fn garyx_db_production_source(source: &Path) -> String {
    let mut files = Vec::new();
    production_rust_files(&source.join("garyx_db"), &mut files);
    files.sort();
    let mut combined = String::new();
    for path in files {
        let body = fs::read_to_string(&path).expect("read garyx db source");
        combined.push_str(body.split("#[cfg(test)]\nmod ").next().unwrap_or(&body));
        combined.push('\n');
    }
    combined
}

fn production_source_file(path: &Path) -> String {
    let body = fs::read_to_string(path).expect("read production Rust source");
    body.split("#[cfg(test)]\nmod ")
        .next()
        .unwrap_or(&body)
        .to_owned()
}

fn production_occurrences(source: &Path, needle: &str) -> Vec<String> {
    let mut files = Vec::new();
    production_rust_files(source, &mut files);
    files.sort();
    let mut occurrences = Vec::new();
    for path in files {
        let body = production_source_file(&path);
        for (line_number, line) in body.lines().enumerate() {
            if line.contains(needle) {
                occurrences.push(format!(
                    "{}:{}:{}",
                    path.strip_prefix(source).unwrap().display(),
                    line_number + 1,
                    line.trim()
                ));
            }
        }
    }
    occurrences
}

fn source_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .unwrap_or_else(|| panic!("missing source inventory start marker: {start}"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("missing source inventory end marker: {end}"))
        .0
}

fn string_slice_constant(source: &str, name: &str) -> Vec<String> {
    let declaration = format!("const {name}: &[&str] = &[");
    let body = source
        .split_once(&declaration)
        .unwrap_or_else(|| panic!("missing writer allowlist constant: {name}"))
        .1
        .split_once("];")
        .unwrap_or_else(|| panic!("unterminated writer allowlist constant: {name}"))
        .0;
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(ToOwned::to_owned)
        .collect()
}

fn assert_writer_allowlist(source: &str, name: &str, expected: &[&str]) {
    assert_eq!(
        string_slice_constant(source, name),
        expected,
        "{name} is a durable existing-record writer contract"
    );
}

#[test]
fn raw_destructive_database_methods_are_crate_private_and_call_site_guarded() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let db = garyx_db_production_source(&source);
    assert!(db.contains("pub(crate) fn archive_thread_record("));
    assert!(db.contains("pub(crate) fn delete_thread_record_with_projections("));
    assert!(!db.contains("pub fn archive_thread_record("));
    assert!(!db.contains("pub fn delete_thread_record_with_projections("));

    assert!(production_calls(&source, ".archive_thread_record(").is_empty());
    let routes = {
        let mut files = vec![source.join("routes.rs")];
        production_rust_files(&source.join("routes"), &mut files);
        files.sort();
        let mut combined = String::new();
        for path in files {
            combined.push_str(&fs::read_to_string(&path).expect("read routes source"));
            combined.push('\n');
        }
        combined
    };
    assert!(!routes.contains(".start_archive("));
    assert!(routes.contains("db.execute_lifecycle_mutation(input)"));
    assert!(routes.contains("db.execute_lifecycle_decision(input)"));
    assert!(routes.contains(".preflight_and_freeze(&request.thread_id"));

    let store = fs::read_to_string(source.join("sqlite_thread_store.rs"))
        .expect("read sqlite thread store source");
    assert_eq!(
        production_calls(&source, ".delete_thread_record_with_projections("),
        vec![
            "sqlite_thread_store.rs:.run_blocking(move |db| db.delete_thread_record_with_projections(&key))"
        ]
    );
    assert!(store.contains(".reserve_delete(self, thread_id)"));
    assert!(store.contains(".abort_and_drain_delete(&reservation)"));
}

#[test]
fn direct_recent_thread_updates_are_prebind_only_and_call_site_guarded() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let db = garyx_db_production_source(&source);
    let production = db.as_str();

    assert_eq!(
        production.matches("\"UPDATE recent_threads\n").count()
            + production.matches("\"UPDATE recent_threads SET").count(),
        4,
        "new direct recent_threads UPDATE paths must either allocate activity_seq or be added as an explicitly reviewed pre-bind-only exception"
    );
    assert!(
        production.contains("\"UPDATE recent_threads\n                SET active_run_id = NULL")
    );
    assert!(
        production.contains("\"UPDATE recent_threads\n                SET thread_type = 'task'")
    );
    assert!(
        production.contains("\"UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2\"")
    );
    assert!(production.contains(
        "RuntimeAssembler invokes this under the data-dir lock before\n        // listener bind"
    ));
    assert!(production.contains(
        "Pre-bind one-shot migration: this direct UPDATE is the sole\n            // backfill allow-list entry"
    ));
    let recent_membership = production
        .split_once("pub(crate) fn migrate_recent_membership_v2")
        .expect("recent membership cutover")
        .1
        .split_once("pub(crate) fn drop_thread_message_routes_v1")
        .expect("next migration method")
        .0;
    assert_eq!(
        recent_membership
            .matches("\"UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2\"")
            .count(),
        1,
        "S5 has exactly one direct recent sequence rewrite"
    );
    assert!(
        recent_membership.contains("Existing members retain their exact frozen relative order")
    );
    assert!(recent_membership.contains("Data and generation-aware marker commit atomically"));
}

#[test]
fn durable_delivery_thread_writer_inventory_is_locked() {
    let gateway_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = gateway_root.parent().expect("repository root");
    let gateway_source = gateway_root.join("src");
    let router_source = repository_root.join("garyx-router/src");

    for removed_escape in [
        "sync_endpoint_delivery_timestamp",
        "upsert_known_channel_endpoint",
    ] {
        assert!(
            production_occurrences(&router_source, removed_escape).is_empty(),
            "removed whole-body endpoint escape reappeared: {removed_escape}"
        );
    }

    let delivery = production_source_file(&router_source.join("router/message/delivery.rs"));
    assert_eq!(
        delivery.matches("self.sync_delivery_timestamp(").count(),
        3,
        "all three delivery-context activity sites must use the endpoint mutator"
    );
    let delivery_sync = source_between(
        &delivery,
        "    async fn sync_delivery_timestamp(",
        "    /// Get the last delivery context for a thread.",
    );
    assert!(delivery_sync.contains(".sync_delivery_timestamp("));
    assert!(delivery_sync.contains("EndpointDeliveryTimestampResult::OwnerChanged"));
    assert!(delivery_sync.contains("EndpointDeliveryTimestampResult::NotFound"));
    assert!(delivery_sync.contains("Err(error)"));
    assert!(delivery_sync.contains("failed to persist delivery timestamp"));
    assert!(!delivery_sync.contains("let _ ="));

    let prepare = production_source_file(&gateway_source.join("application/chat/prepare.rs"));
    assert_writer_allowlist(
        &prepare,
        "PROVIDER_TYPE_PATCH_FIELDS",
        &["provider_type", "updated_at"],
    );
    let provider_type_writer = source_between(
        &prepare,
        "async fn persist_thread_provider_type_if_missing(",
        "pub(crate) async fn prepare_chat_request(",
    );
    assert!(provider_type_writer.contains("PROVIDER_TYPE_PATCH_FIELDS"));
    assert!(provider_type_writer.contains(".patch(thread_id, patch)"));
    assert!(!provider_type_writer.contains(".set("));

    let tasks = production_source_file(&router_source.join("tasks.rs"));
    assert_writer_allowlist(
        &tasks,
        "TASK_CREATION_PATCH_FIELDS",
        &[
            "label",
            "thread_title_source",
            "provider_thread_title",
            "thread_kind",
            "task",
            "updated_at",
        ],
    );
    let task_creation_patch = source_between(
        &tasks,
        "async fn patch_task_creation_record(",
        "#[derive(Debug, thiserror::Error)]",
    );
    assert!(task_creation_patch.contains("TASK_CREATION_PATCH_FIELDS"));
    assert!(!task_creation_patch.contains(".set("));
    let task_creation = source_between(
        &tasks,
        "    pub async fn create_task(",
        "    pub async fn get_task(",
    );
    assert_eq!(
        task_creation.matches("patch_task_creation_record(").count(),
        1
    );
    assert!(!task_creation.contains(".set("));

    let agent_identity = production_source_file(&gateway_source.join("agent_identity.rs"));
    let agent_creation = source_between(
        &agent_identity,
        "pub(crate) async fn create_thread_for_agent_reference(",
        "async fn canonical_thread_options(",
    );
    assert_eq!(agent_creation.matches("create_thread_record(").count(), 1);
    assert!(!agent_creation.contains(".set("));

    let thread_routes = production_source_file(&gateway_source.join("routes/threads.rs"));
    assert_writer_allowlist(
        &thread_routes,
        "IMPORTED_HISTORY_PATCH_FIELDS",
        &[
            "last_user_preview",
            "last_assistant_preview",
            "message_count",
            "history",
            "updated_at",
        ],
    );
    let import_writer = source_between(
        &thread_routes,
        "pub(super) async fn seed_imported_thread_history(",
        "/// Materialize an imported provider transcript",
    );
    assert!(import_writer.contains("IMPORTED_HISTORY_PATCH_FIELDS"));
    assert!(!import_writer.contains(".set("));

    let persistence = production_source_file(
        &repository_root.join("garyx-bridge/src/multi_provider/persistence.rs"),
    );
    assert_writer_allowlist(
        &persistence,
        "RUN_PERSISTENCE_PATCH_FIELDS",
        &[
            "pending_user_inputs",
            "provider_sdk_session_ids",
            "provider_type",
            "provider_key",
            "sdk_session_id",
            "history",
            "last_user_preview",
            "last_assistant_preview",
            "updated_at",
        ],
    );
    assert_eq!(
        persistence.matches("persist_run_record_patch(").count(),
        3,
        "streaming and terminal writers must share the audited patch helper"
    );
    let run_patch = source_between(
        &persistence,
        "async fn persist_run_record_patch(",
        "fn is_internal_dispatch(",
    );
    assert!(run_patch.contains("RUN_PERSISTENCE_PATCH_FIELDS"));

    let model_snapshot = production_source_file(
        &repository_root
            .join("garyx-bridge/src/multi_provider/run_management/persistence_worker.rs"),
    );
    assert_writer_allowlist(
        &model_snapshot,
        "MODEL_RUNTIME_SNAPSHOT_PATCH_FIELDS",
        &["metadata", "updated_at"],
    );
    assert_eq!(
        model_snapshot
            .matches("MODEL_RUNTIME_SNAPSHOT_PATCH_FIELDS")
            .count(),
        2,
        "model snapshot writer must consume its audited allowlist exactly once"
    );
    let model_snapshot_writer = source_between(
        &model_snapshot,
        "pub(super) async fn persist_thread_runtime_snapshot(",
        "pub(super) fn build_pending_input_content(",
    );
    assert!(model_snapshot_writer.contains("MODEL_RUNTIME_SNAPSHOT_PATCH_FIELDS"));
    assert!(model_snapshot_writer.contains("store.patch(thread_id, patch)"));
    assert!(!model_snapshot_writer.contains("store.set("));

    let agent_snapshot = production_source_file(
        &repository_root.join("garyx-bridge/src/multi_provider/lifecycle.rs"),
    );
    assert_writer_allowlist(
        &agent_snapshot,
        "AGENT_RUNTIME_SNAPSHOT_PATCH_FIELDS",
        &["metadata", "updated_at"],
    );
    assert_eq!(
        agent_snapshot
            .matches("AGENT_RUNTIME_SNAPSHOT_PATCH_FIELDS")
            .count(),
        2,
        "agent snapshot writer must consume its audited allowlist exactly once"
    );
    let agent_snapshot_writer = source_between(
        &agent_snapshot,
        "    pub(super) async fn backfill_bound_agent_runtime_metadata(",
        "    pub(super) async fn provider_key_for_agent_id(",
    );
    assert!(agent_snapshot_writer.contains("AGENT_RUNTIME_SNAPSHOT_PATCH_FIELDS"));
    assert!(agent_snapshot_writer.contains("store.patch(thread_id, patch)"));
    assert!(!agent_snapshot_writer.contains("store.set("));

    let provider_title = production_source_file(
        &repository_root.join("garyx-bridge/src/multi_provider/run_management/thread_title.rs"),
    );
    assert_writer_allowlist(
        &provider_title,
        "PROVIDER_THREAD_TITLE_PATCH_FIELDS",
        &[
            "label",
            "provider_thread_title",
            "thread_title_source",
            "updated_at",
        ],
    );
    assert_eq!(
        provider_title
            .matches("PROVIDER_THREAD_TITLE_PATCH_FIELDS")
            .count(),
        2,
        "provider-title writer must consume its audited allowlist exactly once"
    );
    let provider_title_writer = source_between(
        &provider_title,
        "pub(super) async fn persist_provider_thread_title_if_missing(",
        "pub(super) fn forward_applied_thread_title_update(",
    );
    assert!(provider_title_writer.contains("PROVIDER_THREAD_TITLE_PATCH_FIELDS"));
    assert!(provider_title_writer.contains("store.patch(thread_id, patch)"));
    assert!(!provider_title_writer.contains("store.set("));
}
