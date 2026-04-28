use std::fs;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use garyx_gateway::build_router;
use garyx_gateway::server::AppStateBuilder;
use garyx_gateway::skills::SkillsService;
use garyx_models::config::GaryxConfig;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

const TEST_GATEWAY_TOKEN: &str = "skills-api-test-token";

fn write_skill(root: &Path, id: &str, name: &str, description: &str) {
    let skill_dir = root.join(id);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n# {}\n",
            name, description, name
        ),
    )
    .unwrap();
}

fn skill_value<'a>(skills: &'a [Value], id: &str) -> &'a Value {
    skills
        .iter()
        .find(|value| value["id"].as_str() == Some(id))
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn test_router(user_root: &Path, project_root: &Path) -> axum::Router {
    let skills = Arc::new(SkillsService::new(
        user_root.to_path_buf(),
        Some(project_root.to_path_buf()),
    ));
    let mut config = GaryxConfig::default();
    config.gateway.auth_token = TEST_GATEWAY_TOKEN.to_owned();
    let state = AppStateBuilder::new(config)
        .with_skills_service(skills)
        .build();
    build_router(state)
}

fn authed_request() -> axum::http::request::Builder {
    Request::builder().header("authorization", format!("Bearer {TEST_GATEWAY_TOKEN}"))
}

#[tokio::test]
async fn skills_api_lists_user_and_project_skills_with_persisted_state() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");

    write_skill(
        &user_root,
        "local-skill",
        "Local Skill",
        "Available everywhere",
    );
    write_skill(
        &project_root,
        "project-skill",
        "Project Skill",
        "Bound to this repo",
    );
    fs::create_dir_all(&user_root).unwrap();
    fs::write(
        user_root.join(".state.json"),
        json!({ "project-skill": false }).to_string(),
    )
    .unwrap();

    let router = test_router(&user_root, &project_root);
    let response = router
        .oneshot(
            authed_request()
                .uri("/api/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let skills = payload["skills"].as_array().unwrap();

    assert_eq!(skills.len(), 2);
    assert_eq!(skill_value(skills, "local-skill")["enabled"], true);
    assert_eq!(skill_value(skills, "local-skill")["installed"], true);
    assert_eq!(skill_value(skills, "project-skill")["enabled"], false);
    assert!(
        skill_value(skills, "project-skill")["source_path"]
            .as_str()
            .unwrap()
            .contains(".claude/skills/project-skill")
    );
}

#[tokio::test]
async fn skills_api_supports_create_update_toggle_and_delete() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");
    let router = test_router(&user_root, &project_root);

    let create_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "alpha-skill",
                        "name": "Alpha Skill",
                        "description": "Handles alpha workflows",
                        "body": "## Workflow\n\nRun the alpha checklist.\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(create_response.status(), StatusCode::CREATED);
    let created = response_json(create_response).await;
    assert_eq!(created["id"], "alpha-skill");
    assert_eq!(created["enabled"], true);
    assert!(user_root.join("alpha-skill").join("SKILL.md").is_file());
    let created_markdown =
        fs::read_to_string(user_root.join("alpha-skill").join("SKILL.md")).unwrap();
    assert!(created_markdown.contains("## Workflow"));

    let update_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("PATCH")
                .uri("/api/skills/alpha-skill")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "Alpha Prime",
                        "description": "Handles refined workflows",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(update_response.status(), StatusCode::OK);
    let updated = response_json(update_response).await;
    assert_eq!(updated["name"], "Alpha Prime");
    assert_eq!(updated["description"], "Handles refined workflows");
    let skill_markdown =
        fs::read_to_string(user_root.join("alpha-skill").join("SKILL.md")).unwrap();
    assert!(skill_markdown.contains("name: Alpha Prime"));
    assert!(skill_markdown.contains("description: Handles refined workflows"));

    let toggle_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("PATCH")
                .uri("/api/skills/alpha-skill/toggle")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(toggle_response.status(), StatusCode::OK);
    let toggled = response_json(toggle_response).await;
    assert_eq!(toggled["enabled"], false);

    let state_value: Value =
        serde_json::from_str(&fs::read_to_string(user_root.join(".state.json")).unwrap()).unwrap();
    assert_eq!(state_value["alpha-skill"], false);

    let delete_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("DELETE")
                .uri("/api/skills/alpha-skill")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::OK);
    assert!(!user_root.join("alpha-skill").exists());

    let list_response = router
        .oneshot(
            authed_request()
                .uri("/api/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);
    let payload = response_json(list_response).await;
    assert_eq!(
        payload["skills"]
            .as_array()
            .map(|items| items.len())
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn skills_api_rejects_empty_create_body() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");
    let router = test_router(&user_root, &project_root);

    let response = router
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "empty-skill",
                        "name": "Empty Skill",
                        "description": "Should fail",
                        "body": "   ",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "skill content is required");
}

#[tokio::test]
async fn skills_api_rejects_duplicate_and_invalid_ids() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");
    write_skill(&project_root, "shared-skill", "Shared", "Already exists");
    let router = test_router(&user_root, &project_root);

    let duplicate = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "shared-skill",
                        "name": "Shared",
                        "description": "Duplicate",
                        "body": "## Workflow\n\nDuplicate body.\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let invalid = router
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "Bad Id",
                        "name": "Invalid",
                        "description": "Should fail",
                        "body": "## Workflow\n\nInvalid body.\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn skills_api_supports_directory_tree_and_file_editing() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");
    let router = test_router(&user_root, &project_root);

    let create_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "editor-skill",
                        "name": "Editor Skill",
                        "description": "Editable",
                        "body": "## Workflow\n\nEdit this skill.\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let create_entry_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills/editor-skill/entries")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "path": "scripts/read.mjs",
                        "entryType": "file",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_entry_response.status(), StatusCode::CREATED);
    let created_tree = response_json(create_entry_response).await;
    assert_eq!(created_tree["skill"]["id"], "editor-skill");

    let tree_response = router
        .clone()
        .oneshot(
            authed_request()
                .uri("/api/skills/editor-skill/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tree_response.status(), StatusCode::OK);
    let tree_payload = response_json(tree_response).await;
    assert_eq!(tree_payload["entries"][0]["entryType"], "directory");
    assert_eq!(tree_payload["entries"][0]["path"], "scripts");
    assert_eq!(
        tree_payload["entries"][0]["children"][0]["path"],
        "scripts/read.mjs"
    );

    let write_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("PUT")
                .uri("/api/skills/editor-skill/file")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "path": "scripts/read.mjs",
                        "content": "console.log('ok');\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write_response.status(), StatusCode::OK);

    let read_response = router
        .clone()
        .oneshot(
            authed_request()
                .uri("/api/skills/editor-skill/file?path=scripts%2Fread.mjs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_response.status(), StatusCode::OK);
    let file_payload = response_json(read_response).await;
    assert_eq!(file_payload["path"], "scripts/read.mjs");
    assert_eq!(file_payload["content"], "console.log('ok');\n");

    let delete_entry_response = router
        .oneshot(
            authed_request()
                .method("DELETE")
                .uri("/api/skills/editor-skill/entries?path=scripts%2Fread.mjs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_entry_response.status(), StatusCode::OK);
    assert!(
        !user_root
            .join("editor-skill")
            .join("scripts")
            .join("read.mjs")
            .exists()
    );
}

#[tokio::test]
async fn skills_api_returns_preview_payload_for_skill_images() {
    let temp = tempdir().unwrap();
    let user_root = temp.path().join("user-skills");
    let project_root = temp.path().join("workspace").join(".claude").join("skills");
    let router = test_router(&user_root, &project_root);

    let create_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "id": "editor-skill",
                        "name": "Editor Skill",
                        "description": "Editable",
                        "body": "## Workflow\n\nEdit this skill.\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);

    let create_entry_response = router
        .clone()
        .oneshot(
            authed_request()
                .method("POST")
                .uri("/api/skills/editor-skill/entries")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "path": "assets/preview.png",
                        "entryType": "file",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_entry_response.status(), StatusCode::CREATED);

    let image_bytes = b"\x89PNG\r\n\x1a\napi-preview";
    fs::write(
        user_root
            .join("editor-skill")
            .join("assets")
            .join("preview.png"),
        image_bytes,
    )
    .unwrap();

    let read_response = router
        .clone()
        .oneshot(
            authed_request()
                .uri("/api/skills/editor-skill/file?path=assets%2Fpreview.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_response.status(), StatusCode::OK);

    let payload = response_json(read_response).await;
    let expected_base64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    assert_eq!(payload["path"], "assets/preview.png");
    assert_eq!(payload["mediaType"], "image/png");
    assert_eq!(payload["previewKind"], "image");
    assert_eq!(payload["editable"], false);
    assert_eq!(payload["content"], "");
    assert_eq!(
        payload["dataBase64"].as_str(),
        Some(expected_base64.as_str())
    );
}
