//! Reusable agent-loop core.
//!
//! The crate owns the model-neutral loop concepts: conversation messages, tool
//! definitions, pending user input, model requests, loop events, hooks, and
//! compaction. Host applications are responsible for translating their own
//! transcript, configuration, authentication, and streaming-event types at the
//! boundary.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex;

pub mod adapters;
pub mod compaction;
pub mod message;

pub use message::{ConversationMessage, ConversationRole, PendingUserInput};

use compaction::{
    ContextCompactionConfig, ContextCompactionResult, build_compaction_plan,
    compact_messages_with_summary, serialize_messages_for_summary,
};

#[derive(Debug, Error)]
pub enum AgentLoopError {
    #[error("agent loop timed out")]
    Timeout,
    #[error("{0}")]
    Failed(String),
}

impl AgentLoopError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelVendor {
    OpenAi,
    Anthropic,
    Google,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmOutput {
    Text(String),
    ToolCall(LlmToolCall),
}

#[derive(Debug, Clone, Default)]
pub struct LlmResponse {
    pub outputs: Vec<LlmOutput>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub actual_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRequestOptions {
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmRuntimeContext {
    pub env: HashMap<String, String>,
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub instructions: String,
    pub messages: Vec<ConversationMessage>,
    pub tools: Vec<ToolDefinition>,
    pub options: LlmRequestOptions,
    pub runtime: LlmRuntimeContext,
}

#[async_trait]
pub trait LlmAdapter: Send + Sync {
    fn vendor(&self) -> ModelVendor;

    async fn sample(&self, request: LlmRequest) -> Result<LlmResponse, AgentLoopError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecution {
    pub content: Value,
    pub is_error: bool,
    pub terminate: bool,
}

impl ToolExecution {
    pub fn ok(content: Value) -> Self {
        Self {
            content,
            is_error: false,
            terminate: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: json!({ "error": message.into() }),
            is_error: true,
            terminate: false,
        }
    }
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_tool(&self, call: &LlmToolCall) -> ToolExecution;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Debug, Clone)]
pub struct ContextTransformInput {
    pub turn_index: u32,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone)]
pub struct BeforeToolCallInput {
    pub turn_index: u32,
    pub call: LlmToolCall,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BeforeToolCallResult {
    Continue,
    Block(ToolExecution),
}

#[derive(Debug, Clone)]
pub struct AfterToolCallInput {
    pub turn_index: u32,
    pub call: LlmToolCall,
    pub execution: ToolExecution,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone)]
pub struct AgentLoopTurnContext {
    pub turn_index: u32,
    pub messages: Vec<ConversationMessage>,
    pub turn_messages: Vec<ConversationMessage>,
    pub response: String,
    pub tool_results: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentLoopTurnUpdate {
    pub model: Option<String>,
    pub instructions: Option<String>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub options: Option<LlmRequestOptions>,
    pub runtime: Option<LlmRuntimeContext>,
    pub request_timeout: Option<Duration>,
}

#[async_trait]
pub trait AgentLoopHooks: Send + Sync {
    async fn transform_context(
        &self,
        input: ContextTransformInput,
    ) -> Result<Vec<ConversationMessage>, AgentLoopError> {
        Ok(input.messages)
    }

    async fn before_tool_call(
        &self,
        _input: BeforeToolCallInput,
    ) -> Result<BeforeToolCallResult, AgentLoopError> {
        Ok(BeforeToolCallResult::Continue)
    }

    async fn after_tool_call(
        &self,
        input: AfterToolCallInput,
    ) -> Result<ToolExecution, AgentLoopError> {
        Ok(input.execution)
    }

    async fn prepare_next_turn(
        &self,
        _context: AgentLoopTurnContext,
    ) -> Result<Option<AgentLoopTurnUpdate>, AgentLoopError> {
        Ok(None)
    }

    async fn should_stop_after_turn(
        &self,
        _context: AgentLoopTurnContext,
    ) -> Result<bool, AgentLoopError> {
        Ok(false)
    }

    async fn steering_messages(&self) -> Result<Vec<ConversationMessage>, AgentLoopError> {
        Ok(Vec::new())
    }

    async fn follow_up_messages(&self) -> Result<Vec<ConversationMessage>, AgentLoopError> {
        Ok(Vec::new())
    }
}

pub struct NoopAgentLoopHooks;

#[async_trait]
impl AgentLoopHooks for NoopAgentLoopHooks {}

#[derive(Debug, Clone)]
pub struct AgentLoopSession {
    pub sdk_session_id: String,
    pub messages: Vec<ConversationMessage>,
    pub pending_inputs: VecDeque<PendingUserInput>,
    pub interrupted: bool,
}

impl AgentLoopSession {
    pub fn new(sdk_session_id: String) -> Self {
        Self {
            sdk_session_id,
            messages: Vec::new(),
            pending_inputs: VecDeque::new(),
            interrupted: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentLoopRunRequest {
    pub model: String,
    pub instructions: String,
    pub tools: Vec<ToolDefinition>,
    pub options: LlmRequestOptions,
    pub runtime: LlmRuntimeContext,
    pub request_timeout: Duration,
    pub max_tool_iterations: u32,
    pub max_turns: Option<u32>,
    pub queue_mode: QueueMode,
    pub compaction: Option<ContextCompactionConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentLoopEvent {
    SessionBound {
        sdk_session_id: String,
    },
    TurnStart {
        turn_index: u32,
    },
    TurnEnd {
        turn_index: u32,
    },
    ContextTransformed {
        original_messages: usize,
        request_messages: usize,
    },
    Compaction {
        summary: ContextCompactionResult,
    },
    SteeringMessage {
        message: ConversationMessage,
    },
    FollowUpMessage {
        message: ConversationMessage,
    },
    Delta {
        text: String,
    },
    ToolUse {
        message: ConversationMessage,
    },
    ToolExecutionStart {
        tool_use_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolExecutionEnd {
        tool_use_id: String,
        tool_name: String,
        is_error: bool,
    },
    ToolResult {
        message: ConversationMessage,
    },
    UserAck {
        pending_input_id: Option<String>,
    },
    Done,
}

#[derive(Debug, Clone, Default)]
pub struct AgentLoopOutcome {
    pub response: String,
    pub session_messages: Vec<ConversationMessage>,
    pub actual_model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

pub async fn run_agent_loop(
    session: Arc<Mutex<AgentLoopSession>>,
    adapter: &dyn LlmAdapter,
    tool_executor: &dyn ToolExecutor,
    request: AgentLoopRunRequest,
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(AgentLoopEvent) + Send,
) -> Result<AgentLoopOutcome, AgentLoopError> {
    let hooks = NoopAgentLoopHooks;
    run_agent_loop_with_hooks(
        session,
        adapter,
        tool_executor,
        &hooks,
        request,
        cancel,
        &mut emit,
    )
    .await
}

pub async fn run_agent_loop_with_hooks(
    session: Arc<Mutex<AgentLoopSession>>,
    adapter: &dyn LlmAdapter,
    tool_executor: &dyn ToolExecutor,
    hooks: &dyn AgentLoopHooks,
    mut request: AgentLoopRunRequest,
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(AgentLoopEvent) + Send,
) -> Result<AgentLoopOutcome, AgentLoopError> {
    let sdk_session_id = session.lock().await.sdk_session_id.clone();
    emit(AgentLoopEvent::SessionBound { sdk_session_id });

    let mut outcome = AgentLoopOutcome::default();
    let mut iterations = 0u32;
    let mut turn_index = 0u32;
    let max_iterations = request.max_tool_iterations.max(1);

    loop {
        if cancel.load(Ordering::Relaxed) || session.lock().await.interrupted {
            return Err(AgentLoopError::failed("agent loop interrupted"));
        }

        append_hook_messages(
            session.clone(),
            hooks.steering_messages().await?,
            HookMessageKind::Steering,
            &mut emit,
        )
        .await;

        maybe_compact_session(
            session.clone(),
            adapter,
            &request,
            request.compaction.clone(),
            &mut emit,
        )
        .await?;

        let base_messages = { session.lock().await.messages.clone() };
        let messages = hooks
            .transform_context(ContextTransformInput {
                turn_index: turn_index + 1,
                messages: base_messages.clone(),
            })
            .await?;
        if messages.len() != base_messages.len() {
            emit(AgentLoopEvent::ContextTransformed {
                original_messages: base_messages.len(),
                request_messages: messages.len(),
            });
        }
        let llm_request = LlmRequest {
            model: request.model.clone(),
            instructions: request.instructions.clone(),
            messages,
            tools: request.tools.clone(),
            options: request.options.clone(),
            runtime: request.runtime.clone(),
        };
        emit(AgentLoopEvent::TurnStart {
            turn_index: turn_index + 1,
        });
        let model_response = match tokio::time::timeout(
            request.request_timeout,
            adapter.sample(llm_request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(AgentLoopError::Timeout),
        };

        outcome.input_tokens += model_response.input_tokens;
        outcome.output_tokens += model_response.output_tokens;
        if outcome.actual_model.is_none() {
            outcome.actual_model = model_response.actual_model.clone();
        }

        let mut needs_follow_up = false;
        let mut turn_tool_results = Vec::<ConversationMessage>::new();
        let mut turn_messages = Vec::<ConversationMessage>::new();
        for output in model_response.outputs {
            match output {
                LlmOutput::Text(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    outcome.response.push_str(&text);
                    emit(AgentLoopEvent::Delta { text: text.clone() });
                    let message = ConversationMessage::assistant_text(text);
                    session.lock().await.messages.push(message.clone());
                    outcome.session_messages.push(message.clone());
                    turn_messages.push(message);
                }
                LlmOutput::ToolCall(call) => {
                    iterations += 1;
                    if iterations > max_iterations {
                        return Err(AgentLoopError::failed(format!(
                            "agent loop exceeded max_tool_iterations={max_iterations}"
                        )));
                    }
                    let tool_use = ConversationMessage::tool_use(
                        json!({
                            "name": call.name,
                            "arguments": call.arguments,
                        }),
                        Some(call.id.clone()),
                        Some(call.name.clone()),
                    );
                    emit(AgentLoopEvent::ToolUse {
                        message: tool_use.clone(),
                    });
                    session.lock().await.messages.push(tool_use.clone());
                    outcome.session_messages.push(tool_use.clone());
                    turn_messages.push(tool_use);

                    emit(AgentLoopEvent::ToolExecutionStart {
                        tool_use_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    });
                    let execution = match hooks
                        .before_tool_call(BeforeToolCallInput {
                            turn_index: turn_index + 1,
                            call: call.clone(),
                            messages: session.lock().await.messages.clone(),
                        })
                        .await?
                    {
                        BeforeToolCallResult::Continue => tool_executor.execute_tool(&call).await,
                        BeforeToolCallResult::Block(execution) => execution,
                    };
                    let execution = hooks
                        .after_tool_call(AfterToolCallInput {
                            turn_index: turn_index + 1,
                            call: call.clone(),
                            execution,
                            messages: session.lock().await.messages.clone(),
                        })
                        .await?;
                    emit(AgentLoopEvent::ToolExecutionEnd {
                        tool_use_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        is_error: execution.is_error,
                    });
                    let tool_result = ConversationMessage::tool_result(
                        execution.content,
                        Some(call.id),
                        Some(call.name),
                        Some(execution.is_error),
                    );
                    emit(AgentLoopEvent::ToolResult {
                        message: tool_result.clone(),
                    });
                    session.lock().await.messages.push(tool_result.clone());
                    outcome.session_messages.push(tool_result.clone());
                    turn_messages.push(tool_result.clone());
                    turn_tool_results.push(tool_result);
                    needs_follow_up |= !execution.terminate;
                }
            }
        }
        turn_index += 1;
        emit(AgentLoopEvent::TurnEnd { turn_index });

        let turn_context = AgentLoopTurnContext {
            turn_index,
            messages: session.lock().await.messages.clone(),
            turn_messages,
            response: outcome.response.clone(),
            tool_results: turn_tool_results,
        };
        if let Some(update) = hooks.prepare_next_turn(turn_context.clone()).await? {
            update.apply_to(&mut request);
        }
        if hooks.should_stop_after_turn(turn_context).await? {
            break;
        }

        let accepted_pending_input =
            drain_pending_inputs(session.clone(), request.queue_mode, &mut emit).await;
        let accepted_follow_up = if !needs_follow_up && !accepted_pending_input {
            append_hook_messages(
                session.clone(),
                hooks.follow_up_messages().await?,
                HookMessageKind::FollowUp,
                &mut emit,
            )
            .await
        } else {
            false
        };

        if let Some(max_turns) = request.max_turns
            && turn_index >= max_turns
        {
            if needs_follow_up || accepted_pending_input || accepted_follow_up {
                return Err(AgentLoopError::failed(format!(
                    "agent loop exceeded max_turns={max_turns}"
                )));
            }
            break;
        }

        if !needs_follow_up && !accepted_pending_input && !accepted_follow_up {
            break;
        }
    }

    emit(AgentLoopEvent::Done);
    Ok(outcome)
}

impl AgentLoopTurnUpdate {
    fn apply_to(self, request: &mut AgentLoopRunRequest) {
        if let Some(model) = self.model {
            request.model = model;
        }
        if let Some(instructions) = self.instructions {
            request.instructions = instructions;
        }
        if let Some(tools) = self.tools {
            request.tools = tools;
        }
        if let Some(options) = self.options {
            request.options = options;
        }
        if let Some(runtime) = self.runtime {
            request.runtime = runtime;
        }
        if let Some(request_timeout) = self.request_timeout {
            request.request_timeout = request_timeout;
        }
    }
}

enum HookMessageKind {
    Steering,
    FollowUp,
}

async fn append_hook_messages(
    session: Arc<Mutex<AgentLoopSession>>,
    messages: Vec<ConversationMessage>,
    kind: HookMessageKind,
    emit: &mut impl FnMut(AgentLoopEvent),
) -> bool {
    let mut accepted = false;
    for message in messages {
        match kind {
            HookMessageKind::Steering => emit(AgentLoopEvent::SteeringMessage {
                message: message.clone(),
            }),
            HookMessageKind::FollowUp => emit(AgentLoopEvent::FollowUpMessage {
                message: message.clone(),
            }),
        }
        session.lock().await.messages.push(message);
        accepted = true;
    }
    accepted
}

async fn drain_pending_inputs(
    session: Arc<Mutex<AgentLoopSession>>,
    queue_mode: QueueMode,
    emit: &mut impl FnMut(AgentLoopEvent),
) -> bool {
    let mut accepted_pending_input = false;
    loop {
        let pending = { session.lock().await.pending_inputs.pop_front() };
        let Some(pending) = pending else {
            break;
        };
        emit(AgentLoopEvent::UserAck {
            pending_input_id: pending.pending_input_id.clone(),
        });
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text(pending.message));
        accepted_pending_input = true;
        if queue_mode == QueueMode::OneAtATime {
            break;
        }
    }
    accepted_pending_input
}

async fn maybe_compact_session(
    session: Arc<Mutex<AgentLoopSession>>,
    adapter: &dyn LlmAdapter,
    request: &AgentLoopRunRequest,
    config: Option<ContextCompactionConfig>,
    emit: &mut impl FnMut(AgentLoopEvent),
) -> Result<(), AgentLoopError> {
    let Some(config) = config.filter(|value| value.enabled) else {
        return Ok(());
    };
    let messages = { session.lock().await.messages.clone() };
    let Some(plan) = build_compaction_plan(&messages, &config) else {
        return Ok(());
    };
    let summary_prompt = serialize_messages_for_summary(&plan.messages_to_summarize);
    let summary_request = LlmRequest {
        model: request.model.clone(),
        instructions: "You are a Garyx context compaction assistant. Summarize the provided conversation so another model can continue the task. Preserve exact goals, constraints, file paths, tool results, errors, and next steps. Do not answer the conversation."
            .to_owned(),
        messages: vec![ConversationMessage::user_text(format!(
            "<conversation>\n{summary_prompt}\n</conversation>\n\nCreate a concise structured checkpoint summary."
        ))],
        tools: Vec::new(),
        options: request.options.clone(),
        runtime: request.runtime.clone(),
    };
    let summary_response = match tokio::time::timeout(
        request.request_timeout,
        adapter.sample(summary_request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(AgentLoopError::Timeout),
    };
    let summary = summary_response
        .outputs
        .into_iter()
        .filter_map(|output| match output {
            LlmOutput::Text(text) => Some(text),
            LlmOutput::ToolCall(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if summary.trim().is_empty() {
        return Err(AgentLoopError::failed(
            "context compaction completed without a text summary",
        ));
    }
    let Some(result) = compact_messages_with_summary(&messages, summary, &config) else {
        return Ok(());
    };
    session.lock().await.messages = result.messages.clone();
    emit(AgentLoopEvent::Compaction { summary: result });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    use serde_json::json;

    use super::*;

    struct FakeAdapter {
        responses: StdMutex<VecDeque<LlmResponse>>,
        requests: StdMutex<Vec<LlmRequest>>,
    }

    impl FakeAdapter {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: StdMutex::new(VecDeque::from(responses)),
                requests: StdMutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<LlmRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmAdapter for FakeAdapter {
        fn vendor(&self) -> ModelVendor {
            ModelVendor::OpenAi
        }

        async fn sample(&self, request: LlmRequest) -> Result<LlmResponse, AgentLoopError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| AgentLoopError::failed("fake response exhausted"))
        }
    }

    struct FakeTools;

    #[async_trait]
    impl ToolExecutor for FakeTools {
        async fn execute_tool(&self, call: &LlmToolCall) -> ToolExecution {
            ToolExecution::ok(json!({
                "name": call.name,
                "arguments": call.arguments,
            }))
        }
    }

    struct CountingTools {
        calls: StdMutex<Vec<String>>,
    }

    impl CountingTools {
        fn new() -> Self {
            Self {
                calls: StdMutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ToolExecutor for CountingTools {
        async fn execute_tool(&self, call: &LlmToolCall) -> ToolExecution {
            self.calls.lock().unwrap().push(call.name.clone());
            ToolExecution::ok(json!({ "executed": call.name }))
        }
    }

    fn request() -> AgentLoopRunRequest {
        AgentLoopRunRequest {
            model: "test-model".to_owned(),
            instructions: "Act.".to_owned(),
            tools: Vec::new(),
            options: LlmRequestOptions {
                reasoning_effort: Some("medium".to_owned()),
                service_tier: None,
            },
            runtime: LlmRuntimeContext::default(),
            request_timeout: Duration::from_secs(5),
            max_tool_iterations: 4,
            max_turns: None,
            queue_mode: QueueMode::All,
            compaction: None,
        }
    }

    #[tokio::test]
    async fn assistant_text_turn_updates_session_and_events() {
        let adapter = FakeAdapter::new(vec![LlmResponse {
            outputs: vec![LlmOutput::Text("done".to_owned())],
            actual_model: Some("actual".to_owned()),
            input_tokens: 3,
            output_tokens: 2,
        }]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("hello"));
        let mut events = Vec::new();

        let outcome = run_agent_loop(
            session.clone(),
            &adapter,
            &FakeTools,
            request(),
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "done");
        assert_eq!(outcome.actual_model.as_deref(), Some("actual"));
        assert_eq!(outcome.input_tokens, 3);
        assert_eq!(outcome.output_tokens, 2);
        assert!(matches!(events[0], AgentLoopEvent::SessionBound { .. }));
        assert!(events.contains(&AgentLoopEvent::Delta {
            text: "done".to_owned()
        }));
        assert!(events.contains(&AgentLoopEvent::Done));
        assert_eq!(
            session.lock().await.messages.last().unwrap().role_str(),
            "assistant"
        );
    }

    #[tokio::test]
    async fn tool_call_runs_follow_up_request() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::ToolCall(LlmToolCall {
                    id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: json!({ "path": "README.md" }),
                })],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("after tool".to_owned())],
                ..Default::default()
            },
        ]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("read"));

        let outcome = run_agent_loop(
            session,
            &adapter,
            &FakeTools,
            request(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "after tool");
        let requests = adapter.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|message| message.role_str() == "tool_result")
        );
    }

    #[tokio::test]
    async fn queued_input_is_acknowledged_and_sampled_again() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::Text("first".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("second".to_owned())],
                ..Default::default()
            },
        ]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        {
            let mut guard = session.lock().await;
            guard.messages.push(ConversationMessage::user_text("hello"));
            guard
                .pending_inputs
                .push_back(PendingUserInput::text("follow").with_pending_input_id("pending-1"));
        }
        let mut events = Vec::new();

        let outcome = run_agent_loop(
            session,
            &adapter,
            &FakeTools,
            request(),
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "firstsecond");
        assert!(events.contains(&AgentLoopEvent::UserAck {
            pending_input_id: Some("pending-1".to_owned())
        }));
        assert_eq!(adapter.requests().len(), 2);
    }

    struct TailOnlyContextHook;

    #[async_trait]
    impl AgentLoopHooks for TailOnlyContextHook {
        async fn transform_context(
            &self,
            input: ContextTransformInput,
        ) -> Result<Vec<ConversationMessage>, AgentLoopError> {
            Ok(input.messages.into_iter().rev().take(1).collect())
        }
    }

    #[tokio::test]
    async fn transform_context_changes_llm_request_without_mutating_session_history() {
        let adapter = FakeAdapter::new(vec![LlmResponse {
            outputs: vec![LlmOutput::Text("done".to_owned())],
            ..Default::default()
        }]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        {
            let mut guard = session.lock().await;
            guard.messages.push(ConversationMessage::user_text("old"));
            guard
                .messages
                .push(ConversationMessage::user_text("latest"));
        }
        let mut events = Vec::new();

        run_agent_loop_with_hooks(
            session.clone(),
            &adapter,
            &FakeTools,
            &TailOnlyContextHook,
            request(),
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .await
        .unwrap();

        let requests = adapter.requests();
        assert_eq!(requests[0].messages.len(), 1);
        assert_eq!(requests[0].messages[0].text.as_deref(), Some("latest"));
        assert_eq!(session.lock().await.messages.len(), 3);
        assert!(events.contains(&AgentLoopEvent::ContextTransformed {
            original_messages: 2,
            request_messages: 1,
        }));
    }

    struct BlockToolHook;

    #[async_trait]
    impl AgentLoopHooks for BlockToolHook {
        async fn before_tool_call(
            &self,
            input: BeforeToolCallInput,
        ) -> Result<BeforeToolCallResult, AgentLoopError> {
            Ok(BeforeToolCallResult::Block(ToolExecution {
                content: json!({ "blocked": input.call.name }),
                is_error: true,
                terminate: true,
            }))
        }
    }

    #[tokio::test]
    async fn before_tool_call_can_block_execution() {
        let adapter = FakeAdapter::new(vec![LlmResponse {
            outputs: vec![LlmOutput::ToolCall(LlmToolCall {
                id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({ "path": "README.md" }),
            })],
            ..Default::default()
        }]);
        let tools = CountingTools::new();
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("read"));

        let outcome = run_agent_loop_with_hooks(
            session,
            &adapter,
            &tools,
            &BlockToolHook,
            request(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "");
        assert!(tools.calls().is_empty());
        assert_eq!(outcome.session_messages.len(), 2);
        assert_eq!(outcome.session_messages[1].role_str(), "tool_result");
        assert_eq!(outcome.session_messages[1].is_error, Some(true));
        assert_eq!(outcome.session_messages[1].content["blocked"], "read_file");
    }

    struct FollowUpAndUpdateHook {
        sent_follow_up: StdMutex<bool>,
    }

    #[async_trait]
    impl AgentLoopHooks for FollowUpAndUpdateHook {
        async fn prepare_next_turn(
            &self,
            context: AgentLoopTurnContext,
        ) -> Result<Option<AgentLoopTurnUpdate>, AgentLoopError> {
            if context.turn_index == 1 {
                Ok(Some(AgentLoopTurnUpdate {
                    model: Some("test-model-2".to_owned()),
                    options: Some(LlmRequestOptions {
                        reasoning_effort: Some("low".to_owned()),
                        service_tier: None,
                    }),
                    ..Default::default()
                }))
            } else {
                Ok(None)
            }
        }

        async fn follow_up_messages(&self) -> Result<Vec<ConversationMessage>, AgentLoopError> {
            let mut sent = self.sent_follow_up.lock().unwrap();
            if *sent {
                return Ok(Vec::new());
            }
            *sent = true;
            Ok(vec![ConversationMessage::user_text("follow-up")])
        }
    }

    #[tokio::test]
    async fn prepare_next_turn_updates_next_request_and_follow_up_continues_loop() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::Text("first".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("second".to_owned())],
                ..Default::default()
            },
        ]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("hello"));
        let hooks = FollowUpAndUpdateHook {
            sent_follow_up: StdMutex::new(false),
        };
        let mut events = Vec::new();

        let outcome = run_agent_loop_with_hooks(
            session,
            &adapter,
            &FakeTools,
            &hooks,
            request(),
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "firstsecond");
        let requests = adapter.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].model, "test-model");
        assert_eq!(requests[1].model, "test-model-2");
        assert_eq!(requests[1].options.reasoning_effort.as_deref(), Some("low"));
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|message| message.text.as_deref() == Some("follow-up"))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentLoopEvent::FollowUpMessage { message }
                if message.text.as_deref() == Some("follow-up")
        )));
    }

    struct TurnMessagesHook {
        sent_follow_up: StdMutex<bool>,
        turn_message_texts: StdMutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl AgentLoopHooks for TurnMessagesHook {
        async fn prepare_next_turn(
            &self,
            context: AgentLoopTurnContext,
        ) -> Result<Option<AgentLoopTurnUpdate>, AgentLoopError> {
            self.turn_message_texts.lock().unwrap().push(
                context
                    .turn_messages
                    .iter()
                    .filter_map(|message| message.text.clone())
                    .collect(),
            );
            Ok(None)
        }

        async fn follow_up_messages(&self) -> Result<Vec<ConversationMessage>, AgentLoopError> {
            let mut sent = self.sent_follow_up.lock().unwrap();
            if *sent {
                return Ok(Vec::new());
            }
            *sent = true;
            Ok(vec![ConversationMessage::user_text("follow-up")])
        }
    }

    #[tokio::test]
    async fn turn_context_exposes_only_current_turn_messages() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::Text("first".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("second".to_owned())],
                ..Default::default()
            },
        ]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("hello"));
        let hooks = TurnMessagesHook {
            sent_follow_up: StdMutex::new(false),
            turn_message_texts: StdMutex::new(Vec::new()),
        };

        run_agent_loop_with_hooks(
            session,
            &adapter,
            &FakeTools,
            &hooks,
            request(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            hooks.turn_message_texts.lock().unwrap().clone(),
            vec![vec!["first".to_owned()], vec!["second".to_owned()]]
        );
    }

    struct StopAfterFirstTurnHook;

    #[async_trait]
    impl AgentLoopHooks for StopAfterFirstTurnHook {
        async fn should_stop_after_turn(
            &self,
            context: AgentLoopTurnContext,
        ) -> Result<bool, AgentLoopError> {
            Ok(context.turn_index == 1)
        }
    }

    #[tokio::test]
    async fn should_stop_after_turn_exits_before_follow_up_poll() {
        let adapter = FakeAdapter::new(vec![LlmResponse {
            outputs: vec![LlmOutput::Text("only".to_owned())],
            ..Default::default()
        }]);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("hello"));

        let outcome = run_agent_loop_with_hooks(
            session,
            &adapter,
            &FakeTools,
            &StopAfterFirstTurnHook,
            request(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "only");
        assert_eq!(adapter.requests().len(), 1);
    }

    #[tokio::test]
    async fn queue_mode_one_at_a_time_drains_only_one_pending_input_per_turn() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::Text("first".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("second".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("third".to_owned())],
                ..Default::default()
            },
        ]);
        let mut request = request();
        request.queue_mode = QueueMode::OneAtATime;
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        {
            let mut guard = session.lock().await;
            guard.messages.push(ConversationMessage::user_text("hello"));
            guard
                .pending_inputs
                .push_back(PendingUserInput::text("one").with_pending_input_id("pending-1"));
            guard
                .pending_inputs
                .push_back(PendingUserInput::text("two").with_pending_input_id("pending-2"));
        }

        let outcome = run_agent_loop(
            session,
            &adapter,
            &FakeTools,
            request,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "firstsecondthird");
        let requests = adapter.requests();
        assert_eq!(requests.len(), 3);
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|message| message.text.as_deref() == Some("one"))
        );
        assert!(
            !requests[1]
                .messages
                .iter()
                .any(|message| message.text.as_deref() == Some("two"))
        );
        assert!(
            requests[2]
                .messages
                .iter()
                .any(|message| message.text.as_deref() == Some("two"))
        );
    }

    #[tokio::test]
    async fn automatic_compaction_summarizes_and_replaces_session_context_before_sampling() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::Text("summary of older work".to_owned())],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::Text("after compact".to_owned())],
                ..Default::default()
            },
        ]);
        let mut request = request();
        request.compaction = Some(ContextCompactionConfig {
            enabled: true,
            context_window_tokens: 18,
            reserve_tokens: 4,
            keep_recent_tokens: 5,
        });
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        {
            let mut guard = session.lock().await;
            guard.messages.push(ConversationMessage::user_text(
                "old user message that is long",
            ));
            guard.messages.push(ConversationMessage::assistant_text(
                "old assistant message that is long",
            ));
            guard
                .messages
                .push(ConversationMessage::user_text("recent question"));
            guard
                .messages
                .push(ConversationMessage::assistant_text("recent answer"));
        }
        let mut events = Vec::new();

        let outcome = run_agent_loop(
            session.clone(),
            &adapter,
            &FakeTools,
            request,
            Arc::new(AtomicBool::new(false)),
            |event| events.push(event),
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "after compact");
        let requests = adapter.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].instructions.contains("context compaction"));
        assert_eq!(requests[1].messages[0].role_str(), "system");
        assert_eq!(
            requests[1].messages[0].metadata.get("garyx_compaction"),
            Some(&json!(true))
        );
        assert!(
            requests[1].messages[0]
                .text
                .as_deref()
                .unwrap()
                .contains("summary of older work")
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentLoopEvent::Compaction { summary }
                if summary.summary == "summary of older work"
        )));
        assert_eq!(session.lock().await.messages[0].role_str(), "system");
    }

    #[tokio::test]
    async fn max_turns_is_enforced_when_more_work_is_pending() {
        let adapter = FakeAdapter::new(vec![LlmResponse {
            outputs: vec![LlmOutput::ToolCall(LlmToolCall {
                id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({ "path": "README.md" }),
            })],
            ..Default::default()
        }]);
        let mut request = request();
        request.max_turns = Some(1);
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));
        session
            .lock()
            .await
            .messages
            .push(ConversationMessage::user_text("read"));

        let error = run_agent_loop(
            session,
            &adapter,
            &FakeTools,
            request,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .expect_err("follow-up tool result should exceed one-turn budget");

        assert!(error.to_string().contains("max_turns=1"));
    }

    #[tokio::test]
    async fn max_tool_iterations_is_enforced() {
        let adapter = FakeAdapter::new(vec![
            LlmResponse {
                outputs: vec![LlmOutput::ToolCall(LlmToolCall {
                    id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: json!({ "path": "README.md" }),
                })],
                ..Default::default()
            },
            LlmResponse {
                outputs: vec![LlmOutput::ToolCall(LlmToolCall {
                    id: "call-2".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: json!({ "path": "Cargo.toml" }),
                })],
                ..Default::default()
            },
        ]);
        let mut request = request();
        request.max_tool_iterations = 1;
        let session = Arc::new(Mutex::new(AgentLoopSession::new("sid".to_owned())));

        let error = run_agent_loop(
            session,
            &adapter,
            &FakeTools,
            request,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .expect_err("should exceed tool iteration budget");

        assert!(error.to_string().contains("max_tool_iterations=1"));
    }
}
