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

    let binding =
        locate_claude_session_binding(session_id, &projects_dir, ProviderType::ClaudeCode)
            .expect("binding");
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
fn locate_claude_session_binding_treats_legacy_tty_hint_as_claude() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let projects_dir = temp.path().join(".claude").join("projects");
    let project_dir = projects_dir.join("-home-user-projects-workspace");
    fs::create_dir_all(&project_dir).unwrap();
    let session_id = "04b3eff5-fea5-4339-a682-afd3774b7cc9";
    fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        format!(
            "{{\"sessionId\":\"{session_id}\",\"cwd\":\"{}\"}}\n",
            workspace.display()
        ),
    )
    .unwrap();

    let roots = ProviderSessionSearchRoots {
        claude_projects_dir: Some(projects_dir),
        codex_state_db: None,
        codex_session_roots: Vec::new(),
        gemini_tmp_dir: None,
    };
    let binding = locate_local_provider_session_with_roots(
        session_id,
        ProviderType::from_slug("claude_tty"),
        &roots,
    )
    .expect("lookup")
    .expect("binding");

    assert_eq!(binding.provider_type, ProviderType::ClaudeCode);
    assert_eq!(binding.agent_id, "claude");
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
        chats_dir.join("session-2026-04-10T13-12-47dc720f.jsonl"),
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
            codex_state_db: None,
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
            codex_state_db: None,
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
        chats_dir.join("session-2026-04-14T00-00-gemini.jsonl"),
        format!(
            concat!(
                "{{\"sessionId\":\"{}\",\"projectHash\":\"project\",\"startTime\":\"2026-04-14T00:00:00Z\",\"lastUpdated\":\"2026-04-14T00:00:00Z\",\"kind\":\"main\"}}\n",
                "{{\"id\":\"u1\",\"timestamp\":\"2026-04-14T00:00:00Z\",\"type\":\"user\",\"content\":[{{\"text\":\"hello gemini\"}}]}}\n",
                "{{\"$set\":{{\"lastUpdated\":\"2026-04-14T00:00:00Z\"}}}}\n",
                "{{\"id\":\"g1\",\"timestamp\":\"2026-04-14T00:00:01Z\",\"type\":\"gemini\",\"content\":\"\",\"toolCalls\":[{{\"name\":\"search\"}}]}}\n",
                "{{\"id\":\"g2\",\"timestamp\":\"2026-04-14T00:00:02Z\",\"type\":\"gemini\",\"content\":\"hi from gemini\"}}\n",
                "{{\"$set\":{{\"lastUpdated\":\"2026-04-14T00:00:02Z\"}}}}\n"
            ),
            session_id
        ),
    )
    .unwrap();

    let recovered = recover_local_provider_session_with_roots(
        session_id,
        Some(ProviderType::GeminiCli),
        &ProviderSessionSearchRoots {
            claude_projects_dir: None,
            codex_state_db: None,
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
fn list_recent_local_provider_sessions_with_roots_orders_titles_and_filters() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();

    let claude_project = temp.path().join(".claude").join("projects").join("project");
    fs::create_dir_all(&claude_project).unwrap();
    fs::write(
        claude_project.join("claude-old.jsonl"),
        format!(
            concat!(
                "{{\"type\":\"user\",\"timestamp\":\"2026-04-14T00:00:00Z\",\"sessionId\":\"claude-old\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"Claude planning thread\"}}}}\n",
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-04-14T00:00:01Z\",\"sessionId\":\"claude-old\",\"cwd\":\"{}\",\"message\":{{\"role\":\"assistant\",\"content\":\"ok\"}}}}\n"
            ),
            workspace.display(),
            workspace.display()
        ),
    )
    .unwrap();

    let codex_state_db = temp.path().join(".codex").join("state_5.sqlite");
    fs::create_dir_all(codex_state_db.parent().unwrap()).unwrap();
    let codex_connection = rusqlite::Connection::open(&codex_state_db).unwrap();
    codex_connection
        .execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                first_user_message TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER
            );
            "#,
        )
        .unwrap();
    codex_connection
        .execute(
            r#"
            INSERT INTO threads (
                id, rollout_path, updated_at, cwd, title, archived,
                first_user_message, preview, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "codex-new",
                temp.path()
                    .join("rollout-codex-new.jsonl")
                    .display()
                    .to_string(),
                1_780_000_004_i64,
                workspace.display().to_string(),
                "Codex newest task title",
                0_i64,
                "",
                "",
                1_780_000_004_000_i64,
            ],
        )
        .unwrap();

    let gemini_project = temp.path().join(".gemini").join("tmp").join("workspace");
    let gemini_chats = gemini_project.join("chats");
    fs::create_dir_all(&gemini_chats).unwrap();
    fs::write(
        gemini_project.join(".project_root"),
        format!("{}\n", workspace.display()),
    )
    .unwrap();
    fs::write(
        gemini_chats.join("gemini-mid.jsonl"),
        concat!(
            "{\"sessionId\":\"gemini-mid\",\"projectHash\":\"project\",\"startTime\":\"2026-04-14T00:00:00Z\",\"lastUpdated\":\"2026-04-14T00:00:00Z\",\"kind\":\"main\"}\n",
            "{\"id\":\"u1\",\"timestamp\":\"2026-04-14T00:00:03Z\",\"type\":\"user\",\"content\":[{\"text\":\"Gemini middle research\"}]}\n",
            "{\"$set\":{\"lastUpdated\":\"2026-04-14T00:00:03Z\"}}\n",
        ),
    )
    .unwrap();

    let roots = ProviderSessionSearchRoots {
        claude_projects_dir: Some(temp.path().join(".claude").join("projects")),
        codex_state_db: Some(codex_state_db),
        codex_session_roots: Vec::new(),
        gemini_tmp_dir: Some(temp.path().join(".gemini").join("tmp")),
    };

    let recent = list_recent_local_provider_sessions_with_roots(None, 2, &roots);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].provider_hint, "codex");
    assert_eq!(recent[0].session_id, "codex-new");
    assert_eq!(recent[0].title, "Codex newest task title");
    assert_eq!(recent[1].provider_hint, "gemini");
    assert_eq!(recent[1].session_id, "gemini-mid");

    let claude =
        list_recent_local_provider_sessions_with_roots(Some(ProviderType::ClaudeCode), 10, &roots);
    assert_eq!(claude.len(), 1);
    assert_eq!(claude[0].provider_hint, "claude");
    assert_eq!(claude[0].session_id, "claude-old");
    assert_eq!(claude[0].title, "Claude planning thread");
}

#[test]
fn list_recent_codex_sessions_reads_state_db_threads_index() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let state_db = temp.path().join("state_5.sqlite");
    let connection = rusqlite::Connection::open(&state_db).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                first_user_message TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER
            );
            "#,
        )
        .unwrap();
    connection
        .execute(
            r#"
            INSERT INTO threads (
                id, rollout_path, updated_at, cwd, title, archived,
                first_user_message, preview, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "codex-current",
                temp.path().join("rollout-current.jsonl").display().to_string(),
                1_780_000_000_i64,
                workspace.display().to_string(),
                "<garyx_memory_context>\n<instructions>background only</instructions>\n</garyx_memory_context>\n\n<system_instruction>\ninternal routing only\n</system_instruction>\n\n<garyx_thread_metadata>\nthread_id: thread::1\n</garyx_thread_metadata>\n\nReal Codex thread title",
                0_i64,
                "fallback",
                "preview",
                1_780_000_000_123_i64,
            ],
        )
        .unwrap();
    connection
        .execute(
            r#"
            INSERT INTO threads (
                id, rollout_path, updated_at, cwd, title, archived,
                first_user_message, preview, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "codex-archived",
                temp.path()
                    .join("rollout-archived.jsonl")
                    .display()
                    .to_string(),
                1_780_000_001_i64,
                workspace.display().to_string(),
                "Archived should not show",
                1_i64,
                "",
                "",
                1_780_000_001_000_i64,
            ],
        )
        .unwrap();

    let recent = list_recent_local_provider_sessions_with_roots(
        Some(ProviderType::CodexAppServer),
        10,
        &ProviderSessionSearchRoots {
            claude_projects_dir: None,
            codex_state_db: Some(state_db),
            codex_session_roots: Vec::new(),
            gemini_tmp_dir: None,
        },
    );

    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].provider_hint, "codex");
    assert_eq!(recent[0].session_id, "codex-current");
    assert_eq!(recent[0].title, "Real Codex thread title");
    assert_eq!(
        recent[0].workspace_dir,
        fs::canonicalize(&workspace)
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
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
            codex_state_db: None,
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
            codex_state_db: None,
            codex_session_roots: vec![codex_root],
            gemini_tmp_dir: None,
        },
    )
    .expect("filtered lookup")
    .expect("codex binding");
    assert_eq!(binding.provider_type, ProviderType::CodexAppServer);
    assert_eq!(binding.agent_id, "codex");
}
