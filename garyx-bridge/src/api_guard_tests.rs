use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(|value| value.to_str()),
                Some(".git" | "target" | "node_modules" | "tests")
            ) {
                continue;
            }
            rust_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs")
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name == "api_guard_tests.rs" || name.ends_with("tests.rs"))
        {
            files.push(path);
        }
    }
}

#[test]
fn provider_run_streaming_production_call_sites_are_exactly_allow_listed() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bridge crate must be in the workspace root");
    let mut files = Vec::new();
    rust_files(workspace_root, &mut files);
    let mut calls = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).expect("read Rust source");
        // Provider modules keep their unit tests in a terminal cfg(test)
        // module. Those direct SPI tests are not production call sites.
        let production = source.split("#[cfg(test)]\nmod ").next().unwrap_or(&source);
        for line in production.lines() {
            if line.contains(".run_streaming(") {
                calls.push(format!(
                    "{}:{}",
                    path.strip_prefix(workspace_root).unwrap().display(),
                    line.trim()
                ));
            }
        }
    }
    calls.sort();
    assert_eq!(
        calls,
        vec![
            "garyx-bridge/src/run_graph.rs:.run_streaming(&state.run_options, stream_cb)"
                .to_owned(),
            "garyx-bridge/src/run_graph.rs:ctx.provider.run_streaming(&state.run_options, noop).await"
                .to_owned(),
        ],
        "unexpected provider SPI call sites: {calls:#?}"
    );
}

#[test]
fn raw_bridge_run_entrypoints_are_not_production_public_api() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let management = fs::read_to_string(source_root.join("multi_provider/run_management.rs"))
        .expect("read run management");
    assert!(management.contains("pub(crate) async fn start_admitted_run("));
    assert!(management.contains("#[cfg(test)]\n    pub async fn start_agent_run("));
    assert!(management.contains("pub(crate) async fn run_inline_streaming("));

    let lib = fs::read_to_string(source_root.join("lib.rs")).expect("read bridge lib");
    assert!(lib.contains("mod run_graph;"));
    assert!(!lib.contains("pub mod run_graph;"));
}
