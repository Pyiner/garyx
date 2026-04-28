use super::*;

#[test]
fn render_unit_file_contains_exec_start_and_logs() {
    let unit = render_unit_file(
        "0.0.0.0",
        31337,
        Path::new("/home/alice/.garyx/logs"),
        None,
        Path::new("/home/alice/.garyx/env"),
    );
    assert!(unit.contains("Description=Garyx AI Gateway"));
    assert!(unit.contains(
        "ExecStart=/bin/sh -c 'exec \"$(getent passwd %u | cut -d: -f7)\" -lic \"exec garyx gateway run --host 0.0.0.0 --port 31337\"'"
    ));
    assert!(unit.contains("append:/home/alice/.garyx/logs/stdout.log"));
    assert!(unit.contains("append:/home/alice/.garyx/logs/stderr.log"));
    assert!(unit.contains("EnvironmentFile=-/home/alice/.garyx/env"));
    assert!(unit.contains("WantedBy=default.target"));
    assert!(!unit.contains("GARYX_WORKSPACE_ROOT"));
}

#[test]
fn render_unit_file_embeds_workspace_root_when_provided() {
    let unit = render_unit_file(
        "127.0.0.1",
        31337,
        Path::new("/home/alice/.garyx/logs"),
        Some(Path::new("/home/alice/repos/garyx")),
        Path::new("/home/alice/.garyx/env"),
    );
    assert!(unit.contains("Environment=GARYX_WORKSPACE_ROOT=/home/alice/repos/garyx"));
}
