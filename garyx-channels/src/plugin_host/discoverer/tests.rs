use super::*;
use tempfile::TempDir;

fn write_plugin(root: &Path, id: &str, binary_name: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    // Binary is referenced but we're only testing discovery, not
    // spawn, so presence of the file is sufficient.
    std::fs::write(dir.join(binary_name), "").unwrap();
    let body = format!(
        r#"
[plugin]
id = "{id}"
version = "0.1.0"
display_name = "{id}"

[entry]
binary = "./{binary_name}"

[capabilities]
delivery_model = "pull_explicit_ack"
"#
    );
    std::fs::write(dir.join("plugin.toml"), body).unwrap();
}

#[test]
fn discovers_each_subdirectory_with_a_manifest() {
    let root = TempDir::new().unwrap();
    write_plugin(root.path(), "alpha", "alpha-bin");
    write_plugin(root.path(), "beta", "beta-bin");
    // Something unrelated in the root should be ignored.
    std::fs::write(root.path().join("README.md"), "not a plugin").unwrap();

    let outcome = ManifestDiscoverer::new([root.path().to_path_buf()])
        .discover()
        .unwrap();
    let mut ids: Vec<_> = outcome
        .plugins
        .iter()
        .map(|m| m.plugin.id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert!(outcome.errors.is_empty());
}

#[test]
fn missing_root_is_not_an_error() {
    let outcome = ManifestDiscoverer::new([PathBuf::from("/does/not/exist")])
        .discover()
        .unwrap();
    assert!(outcome.plugins.is_empty());
    assert!(outcome.errors.is_empty());
}

#[test]
fn duplicate_id_across_roots_is_rejected() {
    let root1 = TempDir::new().unwrap();
    let root2 = TempDir::new().unwrap();
    write_plugin(root1.path(), "same", "x");
    write_plugin(root2.path(), "same", "y");
    let err = ManifestDiscoverer::new([root1.path().to_path_buf(), root2.path().to_path_buf()])
        .discover()
        .unwrap_err();
    assert!(matches!(err, DiscoveryError::DuplicateId { .. }));
}

#[cfg(unix)]
#[test]
fn from_env_splits_unix_style_path_list() {
    // Two real roots + one non-existent separator-adjacent entry to
    // confirm `split_paths` handles it without panicking.
    let root1 = TempDir::new().unwrap();
    let root2 = TempDir::new().unwrap();
    write_plugin(root1.path(), "left", "l");
    write_plugin(root2.path(), "right", "r");
    let list = format!("{}:{}", root1.path().display(), root2.path().display());

    // Scope the env var mutation so we don't pollute other tests.
    let prev = std::env::var_os("GARYX_PLUGIN_DIR");
    unsafe {
        std::env::set_var("GARYX_PLUGIN_DIR", &list);
    }
    let default_root = TempDir::new().unwrap();
    let outcome = ManifestDiscoverer::from_env(default_root.path())
        .discover()
        .unwrap();
    unsafe {
        match prev {
            Some(v) => std::env::set_var("GARYX_PLUGIN_DIR", v),
            None => std::env::remove_var("GARYX_PLUGIN_DIR"),
        }
    }

    let mut ids: Vec<_> = outcome
        .plugins
        .iter()
        .map(|m| m.plugin.id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["left".to_owned(), "right".to_owned()]);
}

#[test]
fn malformed_manifest_is_collected_not_fatal() {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), "not = valid [toml").unwrap();
    // Plus a good one alongside so we know the scan continues.
    write_plugin(root.path(), "good", "g");

    let outcome = ManifestDiscoverer::new([root.path().to_path_buf()])
        .discover()
        .unwrap();
    assert_eq!(outcome.plugins.len(), 1);
    assert_eq!(outcome.plugins[0].plugin.id, "good");
    assert_eq!(outcome.errors.len(), 1);
}
