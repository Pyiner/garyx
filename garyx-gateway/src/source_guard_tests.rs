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
