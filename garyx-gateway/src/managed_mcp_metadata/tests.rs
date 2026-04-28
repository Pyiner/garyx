use super::*;
use serde_json::json;

#[test]
fn inject_managed_mcp_servers_merges_existing_runtime_servers() {
    let mut servers = HashMap::new();
    servers.insert(
        "managed".to_owned(),
        McpServerConfig {
            command: "python3".to_owned(),
            args: vec!["managed.py".to_owned()],
            env: HashMap::new(),
            working_dir: None,
            ..Default::default()
        },
    );

    let mut metadata = HashMap::from([(
        "remote_mcp_servers".to_owned(),
        json!({
            "runtime": {
                "command": "python3",
                "args": ["runtime.py"],
                "enabled": true
            }
        }),
    )]);

    inject_managed_mcp_servers(&servers, &mut metadata);

    assert_eq!(
        metadata["remote_mcp_servers"]["managed"]["args"],
        json!(["managed.py"])
    );
    assert_eq!(
        metadata["remote_mcp_servers"]["runtime"]["args"],
        json!(["runtime.py"])
    );
}

#[test]
fn inject_managed_mcp_servers_skips_disabled_servers() {
    let mut servers = HashMap::new();
    servers.insert(
        "disabled".to_owned(),
        McpServerConfig {
            command: "python3".to_owned(),
            args: vec!["disabled.py".to_owned()],
            env: HashMap::new(),
            enabled: false,
            working_dir: None,
            ..Default::default()
        },
    );

    let mut metadata = HashMap::new();
    inject_managed_mcp_servers(&servers, &mut metadata);

    assert!(metadata.is_empty());
}
