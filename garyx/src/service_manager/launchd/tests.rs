use super::*;

#[test]
fn validate_service_uid_rejects_root() {
    let err = validate_service_uid("0").expect_err("root should be rejected");
    assert!(err.to_string().contains("do not run them with sudo"));
}

#[test]
fn validate_service_uid_accepts_login_user() {
    validate_service_uid("501").expect("login user uid should be accepted");
}

#[test]
fn candidate_install_domains_prefers_gui_for_aqua_session() {
    assert_eq!(
        candidate_install_domains_for("501", true, true),
        vec!["gui/501".to_owned(), "user/501".to_owned()]
    );
}

#[test]
fn candidate_install_domains_prefers_existing_gui_domain_from_background_session() {
    assert_eq!(
        candidate_install_domains_for("501", false, true),
        vec!["gui/501".to_owned(), "user/501".to_owned()]
    );
}

#[test]
fn candidate_install_domains_uses_user_domain_when_no_gui_domain_exists() {
    assert_eq!(
        candidate_install_domains_for("501", false, false),
        vec!["user/501".to_owned()]
    );
}

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
        "exec \\&quot;/opt/homebrew/bin/garyx\\&quot; gateway run --host \\&quot;0.0.0.0\\&quot; --port 31337"
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
    // The unit must not pin itself to the Aqua (GUI) session: the agent is a
    // headless HTTP server and is bootstrapped into an explicit domain
    // (gui/<uid> on desktop, user/<uid> over SSH). An Aqua limit would stop it
    // loading in the per-user domain that SSH / headless logins must use.
    assert!(!plist.contains("LimitLoadToSessionType"));
    assert!(!plist.contains("Aqua"));
}

#[test]
fn render_launch_agent_plist_quotes_bracketed_ipv6_host() {
    // `[::]` unquoted is a glob pattern inside the nested `-lic "..."`
    // command; interactive zsh (`nomatch`) exits 1 on it and the agent
    // crash-loops. The host must render inside escaped double quotes.
    let plist = render_launch_agent_plist(
        Path::new("/opt/homebrew/bin/garyx"),
        "[::]",
        31337,
        Path::new("/tmp/stdout.log"),
        Path::new("/tmp/stderr.log"),
        None,
    );
    assert!(plist.contains("gateway run --host \\&quot;[::]\\&quot; --port 31337"));
    assert!(!plist.contains("--host [::]"));
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
