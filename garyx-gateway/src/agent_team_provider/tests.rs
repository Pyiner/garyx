use super::*;
use garyx_models::ProviderType;
use garyx_router::{InMemoryThreadStore, ThreadEnsureOptions, ThreadStore};
struct NoopThreadCreator;

#[async_trait]
impl ThreadCreator for NoopThreadCreator {
    async fn create_thread(
        &self,
        _thread_store: Arc<dyn ThreadStore>,
        _options: ThreadEnsureOptions,
    ) -> Result<(String, Value), String> {
        unreachable!("test does not create child threads")
    }
}

async fn test_dispatcher() -> GatewaySubAgentDispatcher {
    let custom_agents = Arc::new(CustomAgentStore::new());
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "planner".to_owned(),
            display_name: "Planner".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: String::new(),
            system_prompt: "Plan work.".to_owned(),
        })
        .await
        .expect("planner agent");
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: String::new(),
            system_prompt: "Review work.".to_owned(),
        })
        .await
        .expect("reviewer agent");

    GatewaySubAgentDispatcher::new(
        Weak::new(),
        Arc::new(InMemoryThreadStore::new()),
        Arc::new(NoopThreadCreator),
        custom_agents,
    )
}

#[tokio::test]
async fn child_context_notice_explains_mention_handoff_semantics_for_all_children() {
    let dispatcher = test_dispatcher().await;
    let metadata = AgentTeamChildThreadMetadata {
        team_id: "team::demo".to_owned(),
        team_display_name: "Demo Team".to_owned(),
        group_thread_id: "thread::group".to_owned(),
        workflow_text:
            "Planner drives the outline, reviewer only speaks when explicitly asked to review."
                .to_owned(),
        child_agent_id: "reviewer".to_owned(),
        leader_agent_id: "planner".to_owned(),
        member_agent_ids: vec!["planner".to_owned(), "reviewer".to_owned()],
        context_version: AGENT_TEAM_CHILD_CONTEXT_VERSION,
        initial_context_injected_at: None,
    };

    let notice = dispatcher
        .build_initial_child_context_notice(&metadata)
        .await;

    assert!(
        notice.contains("An @ mention means you are explicitly handing the floor to that teammate"),
        "notice should explain that @ means the mentioned teammate must speak, got:\n{notice}"
    );
    assert!(
        notice.contains(
            "Do NOT use @ mentions for mere references, summaries, or indirect discussion"
        ),
        "notice should forbid @ for plain references, got:\n{notice}"
    );
    assert!(
        notice.contains("write their plain name without `@[DisplayName](agent_id)`"),
        "notice should tell agents to use plain names for non-handoffs, got:\n{notice}"
    );
    assert!(
        notice.contains("Team workflow: Planner drives the outline, reviewer only speaks when explicitly asked to review."),
        "notice should include team workflow text, got:\n{notice}"
    );
    assert!(
        notice.contains("Examples for this team: `@[Planner](planner)`."),
        "notice should include concrete mention examples for non-speaking peers, got:\n{notice}"
    );
}
