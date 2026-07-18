//! Skills CRUD and skill-file handlers.

use super::*;

#[derive(Deserialize)]
pub struct CreateSkillBody {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct UpdateSkillBody {
    pub name: String,
    pub description: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileParams {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSkillFileBody {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkillEntryBody {
    pub path: String,
    pub entry_type: String,
}

/// GET /api/skills - list skills from local and project registries.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.ops.skills.list_skills() {
        Ok(skills) => (StatusCode::OK, Json(json!({ "skills": skills }))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// POST /api/skills - create a new local skill under ~/.garyx/skills.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSkillBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .create_skill(&body.id, &body.name, &body.description, &body.body)
    {
        Ok(skill) => (StatusCode::CREATED, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PATCH /api/skills/:id - update skill metadata in SKILL.md frontmatter.
pub async fn update_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSkillBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .update_skill(&id, &body.name, &body.description)
    {
        Ok(skill) => (StatusCode::OK, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PATCH /api/skills/:id/toggle - flip enabled state.
pub async fn toggle_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.toggle_skill(&id) {
        Ok(skill) => (StatusCode::OK, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// DELETE /api/skills/:id - remove a skill directory.
pub async fn delete_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.delete_skill(&id) {
        Ok(()) => (StatusCode::OK, Json(json!({ "deleted": true, "id": id }))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// GET /api/skills/:id/tree - list all files/directories inside one skill.
pub async fn skill_tree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.skill_editor_state(&id) {
        Ok(editor) => (StatusCode::OK, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// GET /api/skills/:id/file - read one skill file as editable text or preview payload.
pub async fn read_skill_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<SkillFileParams>,
) -> impl IntoResponse {
    match state.ops.skills.read_skill_file(&id, &params.path) {
        Ok(document) => (StatusCode::OK, Json(json!(document))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PUT /api/skills/:id/file - save one editable text file inside a skill directory.
pub async fn write_skill_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<WriteSkillFileBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .write_skill_file(&id, &body.path, &body.content)
    {
        Ok(document) => (StatusCode::OK, Json(json!(document))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// POST /api/skills/:id/entries - create a file or directory inside a skill.
pub async fn create_skill_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateSkillEntryBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .create_skill_entry(&id, &body.path, &body.entry_type)
    {
        Ok(editor) => (StatusCode::CREATED, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// DELETE /api/skills/:id/entries?path=... - remove one file or directory inside a skill.
pub async fn delete_skill_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<SkillFileParams>,
) -> impl IntoResponse {
    match state.ops.skills.delete_skill_entry(&id, &params.path) {
        Ok(editor) => (StatusCode::OK, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

pub(super) fn skill_error_response(error: SkillStoreError) -> (StatusCode, Json<Value>) {
    match error {
        SkillStoreError::Validation(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
        }
        SkillStoreError::AlreadyExists(message) => {
            (StatusCode::CONFLICT, Json(json!({ "error": message })))
        }
        SkillStoreError::NotFound(message) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": message })))
        }
        SkillStoreError::Io(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}
