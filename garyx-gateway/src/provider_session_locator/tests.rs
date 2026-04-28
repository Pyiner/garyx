use super::*;

#[test]
fn locate_claude_session_binding_reads_project_cwd_from_transcript() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("novel");
    fs::create_dir_all(&workspace).unwrap();
    let projects_dir = temp.path().join(".claude").join("projects");
    let project_dir = projects_dir.join("-home-user-projects-novel");
    fs::create_dir_all(&project_dir).unwrap();
    let session_id = "04b3eff5-fea5-4339-a682-afd3774b7cc8";
    fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        format!(
            "{{\"sessionId\":\"{session_id}\",\"cwd\":\"{}\"}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let binding = locate_claude_session_binding(session_id, &projects_dir).expect("binding");
    assert_eq!(binding.provider_type, ProviderType::ClaudeCode);
    assert_eq!(binding.agent_id, "claude");
    assert_eq!(
        binding.workspace_dir,
        fs::canonicalize(&workspace)
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
}

#[test]
fn locate_codex_session_binding_reads_cwd_from_session_meta() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let sessions_root = temp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("04")
        .join("13");
    fs::create_dir_all(&sessions_root).unwrap();
    let session_id = "019d6c71-6511-7643-8c2d-c4b33fcddc3f";
    fs::write(
        sessions_root.join(format!("rollout-2026-04-13T00-00-00-{session_id}.jsonl")),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"{}\"}}}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let binding =
        locate_codex_session_binding(session_id, &[temp.path().join(".codex").join("sessions")])
            .expect("binding");
    assert_eq!(binding.provider_type, ProviderType::CodexAppServer);
    assert_eq!(binding.agent_id, "codex");
    assert_eq!(
        binding.workspace_dir,
        fs::canonicalize(&workspace)
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
}

#[test]
fn locate_gemini_session_binding_reads_project_root() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("novel");
    fs::create_dir_all(&workspace).unwrap();
    let project_dir = temp.path().join(".gemini").join("tmp").join("novel");
    let chats_dir = project_dir.join("chats");
    fs::create_dir_all(&chats_dir).unwrap();
    fs::write(
        project_dir.join(".project_root"),
        format!("{}\n", workspace.display()),
    )
    .unwrap();
    let session_id = "47dc720f-5a99-4ca6-9904-11513e92af91";
    fs::write(
        chats_dir.join("session-2026-04-10T13-12-47dc720f.json"),
        format!("{{\"sessionId\":\"{session_id}\"}}"),
    )
    .unwrap();

    let binding =
        locate_gemini_session_binding(session_id, &temp.path().join(".gemini").join("tmp"))
            .expect("binding");
    assert_eq!(binding.provider_type, ProviderType::GeminiCli);
    assert_eq!(binding.agent_id, "gemini");
    assert_eq!(
        binding.workspace_dir,
        fs::canonicalize(&workspace)
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
}

#[test]
fn recover_local_provider_session_with_roots_imports_claude_messages() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("novel");
    fs::create_dir_all(&workspace).unwrap();
    let projects_dir = temp.path().join(".claude").join("projects");
    let project_dir = projects_dir.join("project");
    fs::create_dir_all(&project_dir).unwrap();
    let session_id = "claude-session";
    fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        format!(
            concat!(
                "{{\"type\":\"user\",\"timestamp\":\"2026-04-14T00:00:00Z\",\"sessionId\":\"{}\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}}}}\n",
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-04-14T00:00:01Z\",\"sessionId\":\"{}\",\"cwd\":\"{}\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"world\"}},{{\"type\":\"tool_use\",\"name\":\"shell\"}}]}}}}\n"
            ),
            session_id,
            workspace.display(),
            session_id,
            workspace.display()
        ),
    )
    .unwrap();

    let recovered = recover_local_provider_session_with_roots(
        session_id,
        Some(ProviderType::ClaudeCode),
        &ProviderSessionSearchRoots {
            claude_projects_dir: Some(projects_dir),
            codex_session_roots: Vec::new(),
            gemini_tmp_dir: None,
        },
    )
    .expect("recover")
    .expect("claude recovery");

    assert_eq!(recovered.binding.agent_id, "claude");
    assert_eq!(recovered.messages.len(), 2);
    assert_eq!(recovered.messages[0]["role"], "user");
    assert_eq!(recovered.messages[0]["content"], "hello");
    assert_eq!(recovered.messages[1]["role"], "assistant");
    assert_eq!(recovered.messages[1]["content"], "world");
}

#[test]
fn recover_local_provider_session_with_roots_imports_codex_messages() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let sessions_root = temp.path().join(".codex").join("sessions");
    fs::create_dir_all(&sessions_root).unwrap();
    let session_id = "codex-session";
    fs::write(
        sessions_root.join(format!("rollout-{session_id}.jsonl")),
        format!(
            concat!(
                "{{\"timestamp\":\"2026-04-14T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{}\"}}}}\n",
                "{{\"timestamp\":\"2026-04-14T00:00:01Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"<environment_context>ignored</environment_context>\"}}]}}}}\n",
                "{{\"timestamp\":\"2026-04-14T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"hello codex\"}}}}\n",
                "{{\"timestamp\":\"2026-04-14T00:00:03Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"hi from codex\"}}]}}}}\n",
                "{{\"timestamp\":\"2026-04-14T00:00:04Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"name\":\"shell\"}}}}\n"
            ),
            session_id,
            workspace.display()
        ),
    )
    .unwrap();

    let recovered = recover_local_provider_session_with_roots(
        session_id,
        Some(ProviderType::CodexAppServer),
        &ProviderSessionSearchRoots {
            claude_projects_dir: None,
            codex_session_roots: vec![sessions_root],
            gemini_tmp_dir: None,
        },
    )
    .expect("recover")
    .expect("codex recovery");

    assert_eq!(recovered.binding.agent_id, "codex");
    assert_eq!(recovered.messages.len(), 2);
    assert_eq!(recovered.messages[0]["role"], "user");
    assert_eq!(recovered.messages[0]["content"], "hello codex");
    assert_eq!(recovered.messages[1]["role"], "assistant");
    assert_eq!(recovered.messages[1]["content"], "hi from codex");
}

#[test]
fn recover_local_provider_session_with_roots_imports_gemini_messages() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("novel");
    fs::create_dir_all(&workspace).unwrap();
    let project_dir = temp.path().join(".gemini").join("tmp").join("novel");
    let chats_dir = project_dir.join("chats");
    fs::create_dir_all(&chats_dir).unwrap();
    fs::write(
        project_dir.join(".project_root"),
        format!("{}\n", workspace.display()),
    )
    .unwrap();
    let session_id = "gemini-session";
    fs::write(
        chats_dir.join("session-2026-04-14T00-00-gemini.json"),
        json!({
            "sessionId": session_id,
            "messages": [
                {
                    "type": "user",
                    "timestamp": "2026-04-14T00:00:00Z",
                    "content": "hello gemini"
                },
                {
                    "type": "gemini",
                    "timestamp": "2026-04-14T00:00:01Z",
                    "content": "",
                    "toolCalls": [{"name": "search"}]
                },
                {
                    "type": "gemini",
                    "timestamp": "2026-04-14T00:00:02Z",
                    "content": "hi from gemini"
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let recovered = recover_local_provider_session_with_roots(
        session_id,
        Some(ProviderType::GeminiCli),
        &ProviderSessionSearchRoots {
            claude_projects_dir: None,
            codex_session_roots: Vec::new(),
            gemini_tmp_dir: Some(temp.path().join(".gemini").join("tmp")),
        },
    )
    .expect("recover")
    .expect("gemini recovery");

    assert_eq!(recovered.binding.agent_id, "gemini");
    assert_eq!(recovered.messages.len(), 2);
    assert_eq!(recovered.messages[0]["role"], "user");
    assert_eq!(recovered.messages[0]["content"], "hello gemini");
    assert_eq!(recovered.messages[1]["role"], "assistant");
    assert_eq!(recovered.messages[1]["content"], "hi from gemini");
}

#[test]
fn locate_local_provider_session_with_roots_rejects_ambiguous_matches() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("shared");
    fs::create_dir_all(&workspace).unwrap();
    let session_id = "same-session-id";

    let claude_project = temp.path().join(".claude").join("projects").join("project");
    fs::create_dir_all(&claude_project).unwrap();
    fs::write(
        claude_project.join(format!("{session_id}.jsonl")),
        format!(
            "{{\"sessionId\":\"{session_id}\",\"cwd\":\"{}\"}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let codex_root = temp.path().join(".codex").join("sessions");
    fs::create_dir_all(&codex_root).unwrap();
    fs::write(
        codex_root.join(format!("rollout-{session_id}.jsonl")),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"{}\"}}}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let error = locate_local_provider_session_with_roots(
        session_id,
        None,
        &ProviderSessionSearchRoots {
            claude_projects_dir: Some(temp.path().join(".claude").join("projects")),
            codex_session_roots: vec![codex_root],
            gemini_tmp_dir: None,
        },
    )
    .expect_err("ambiguous");
    assert!(error.contains("matches multiple local providers"));
}

#[test]
fn locate_local_provider_session_with_roots_filters_by_provider_hint() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("shared");
    fs::create_dir_all(&workspace).unwrap();
    let session_id = "same-session-id";

    let claude_project = temp.path().join(".claude").join("projects").join("project");
    fs::create_dir_all(&claude_project).unwrap();
    fs::write(
        claude_project.join(format!("{session_id}.jsonl")),
        format!(
            "{{\"sessionId\":\"{session_id}\",\"cwd\":\"{}\"}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let codex_root = temp.path().join(".codex").join("sessions");
    fs::create_dir_all(&codex_root).unwrap();
    fs::write(
        codex_root.join(format!("rollout-{session_id}.jsonl")),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"{}\"}}}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let binding = locate_local_provider_session_with_roots(
        session_id,
        Some(ProviderType::CodexAppServer),
        &ProviderSessionSearchRoots {
            claude_projects_dir: Some(temp.path().join(".claude").join("projects")),
            codex_session_roots: vec![codex_root],
            gemini_tmp_dir: None,
        },
    )
    .expect("filtered lookup")
    .expect("codex binding");
    assert_eq!(binding.provider_type, ProviderType::CodexAppServer);
    assert_eq!(binding.agent_id, "codex");
}
