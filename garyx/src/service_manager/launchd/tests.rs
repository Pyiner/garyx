use super::*;

#[test]
fn render_launch_agent_plist_uses_expected_label_and_program() {
    let plist = render_launch_agent_plist(
        Path::new("/opt/homebrew/bin/garyx"),
        "0.0.0.0",
        31337,
        Path::new("/tmp/stdout.log"),
        Path::new("/tmp/stderr.log"),
        None,
    );
    assert!(plist.contains("<string>com.garyx.agent</string>"));
    assert!(plist.contains("<string>/bin/sh</string>"));
    assert!(plist.contains("<string>-c</string>"));
    // The command resolves the user's login shell via dscl, re-enters it
    // as login+interactive, then execs the pinned garyx binary.
    assert!(plist.contains("dscl . -read /Users/$(id -un) UserShell"));
    assert!(plist.contains("-lic"));
    assert!(plist.contains(
        "exec \\&quot;/opt/homebrew/bin/garyx\\&quot; gateway run --host 0.0.0.0 --port 31337"
    ));
    assert!(!plist.contains("<key>WorkingDirectory</key>"));
    assert!(!plist.contains("GARYX_WORKSPACE_ROOT"));
    assert!(plist.contains("/tmp/stdout.log"));
    assert!(plist.contains("/tmp/stderr.log"));
    // Ensure we raise NumberOfFiles above the 1024 default so child
    // processes like the `claude` CLI don't hit "low max file
    // descriptors" on startup.
    assert!(plist.contains("<key>SoftResourceLimits</key>"));
    assert!(plist.contains("<key>HardResourceLimits</key>"));
    assert!(plist.contains("<integer>65536</integer>"));
    assert!(!plist.contains("<integer>1024</integer>"));
}

#[test]
fn render_launch_agent_plist_embeds_workspace_root_when_provided() {
    let plist = render_launch_agent_plist(
        Path::new("/opt/homebrew/bin/garyx"),
        "0.0.0.0",
        31337,
        Path::new("/tmp/stdout.log"),
        Path::new("/tmp/stderr.log"),
        Some(Path::new("/Users/me/repos/garyx")),
    );
    assert!(plist.contains("<key>GARYX_WORKSPACE_ROOT</key>"));
    assert!(plist.contains("<string>/Users/me/repos/garyx</string>"));
}
