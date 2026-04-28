use super::*;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// -- Mock provider --

struct MockProvider {
    ready: AtomicBool,
    should_fail: bool,
    response_text: String,
}

impl MockProvider {
    fn ok(text: &str) -> Self {
        Self {
            ready: AtomicBool::new(true),
            should_fail: false,
            response_text: text.to_owned(),
        }
    }

    fn failing() -> Self {
        Self {
            ready: AtomicBool::new(true),
            should_fail: true,
            response_text: String::new(),
        }
    }

    fn not_ready() -> Self {
        Self {
            ready: AtomicBool::new(false),
            should_fail: false,
            response_text: String::new(),
        }
    }
}

#[async_trait::async_trait]
impl AgentLoopProvider for MockProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if self.should_fail {
            return Err(BridgeError::RunFailed("mock failure".to_owned()));
        }
        // Simulate streaming: emit two chunks and one done event.
        let char_mid = self.response_text.chars().count() / 2;
        let split = self
            .response_text
            .char_indices()
            .nth(char_mid)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        on_chunk(StreamEvent::Delta {
            text: self.response_text[..split].to_owned(),
        });
        on_chunk(StreamEvent::Delta {
            text: self.response_text[split..].to_owned(),
        });
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "mock-run".into(),
            thread_id: options.thread_id.clone(),
            response: self.response_text.clone(),
            session_messages: Vec::new(),
            sdk_session_id: Some("sdk-123".into()),
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 100,
            output_tokens: 50,
            cost: 0.005,
            duration_ms: 200,
        })
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

fn make_options(message: &str) -> ProviderRunOptions {
    ProviderRunOptions {
        thread_id: "test::session".to_owned(),
        message: message.to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    }
}

fn make_state(options: ProviderRunOptions) -> RunGraphState {
    RunGraphState::new(
        "run-1".to_owned(),
        options.thread_id.clone(),
        "provider-key".to_owned(),
        options,
    )
}

// -- Tests --

#[tokio::test]
async fn test_successful_run_no_streaming() {
    let provider = MockProvider::ok("Hello from mock!");
    let options = make_options("hi");
    let mut state = make_state(options);

    let result = execute_agent_run(&provider, &mut state, None).await;

    assert!(result.is_ok());
    let res = result.unwrap();
    assert_eq!(res.response, "Hello from mock!");
    assert!(res.success);
    assert_eq!(res.input_tokens, 100);
    assert_eq!(res.output_tokens, 50);

    // State should be Done with Completed metrics
    assert_eq!(state.phase, RunPhase::Done);
    assert_eq!(state.metrics.state, RunState::Completed);
    assert!(state.metrics.start_time.is_some());
    assert!(state.metrics.end_time.is_some());
    assert!(state.metrics.duration_ms() >= 0);
    assert!((state.metrics.cost_usd - 0.005).abs() < f64::EPSILON);
    assert_eq!(state.response, Some("Hello from mock!".to_owned()));
    assert!(state.error.is_none());
}

#[tokio::test]
async fn test_successful_run_with_streaming() {
    let provider = MockProvider::ok("Hello world!");
    let options = make_options("hi");
    let mut state = make_state(options);

    let chunk_count = Arc::new(AtomicU32::new(0));
    let chunk_count_cb = chunk_count.clone();

    let cb: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |_event| {
        chunk_count_cb.fetch_add(1, Ordering::Relaxed);
    });

    let result = execute_agent_run(&provider, &mut state, Some(cb)).await;

    assert!(result.is_ok());
    assert_eq!(state.phase, RunPhase::Done);
    assert_eq!(state.metrics.state, RunState::Completed);

    // Should have received 3 events: delta, delta, done.
    assert_eq!(chunk_count.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn test_run_failure() {
    let provider = MockProvider::failing();
    let options = make_options("hi");
    let mut state = make_state(options);

    let result = execute_agent_run(&provider, &mut state, None).await;

    assert!(result.is_err());
    assert_eq!(state.phase, RunPhase::Done);
    assert_eq!(state.metrics.state, RunState::Error);
    assert!(state.metrics.error_message.is_some());
    assert!(state.error.is_some());
    assert!(state.metrics.end_time.is_some());
}

#[tokio::test]
async fn test_provider_not_ready() {
    let provider = MockProvider::not_ready();
    let options = make_options("hi");
    let mut state = make_state(options);

    let result = execute_agent_run(&provider, &mut state, None).await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BridgeError::ProviderNotReady));
    assert_eq!(state.phase, RunPhase::Done);
    assert_eq!(state.metrics.state, RunState::Error);
}

#[tokio::test]
async fn test_streaming_failure() {
    let provider = MockProvider::failing();
    let options = make_options("hi");
    let mut state = make_state(options);

    let cb: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(|_event| {});

    let result = execute_agent_run(&provider, &mut state, Some(cb)).await;

    assert!(result.is_err());
    assert_eq!(state.metrics.state, RunState::Error);
}

#[test]
fn test_run_metrics_defaults() {
    let m = RunMetrics::default();
    assert_eq!(m.state, RunState::Pending);
    assert!(m.start_time.is_none());
    assert!(m.first_token_time.is_none());
    assert!(m.end_time.is_none());
    assert_eq!(m.input_tokens, 0);
    assert_eq!(m.output_tokens, 0);
    assert!((m.cost_usd - 0.0).abs() < f64::EPSILON);
    assert!(m.error_message.is_none());
    assert_eq!(m.duration_ms(), 0);
    assert_eq!(m.time_to_first_token_ms(), 0);
}

#[test]
fn test_run_metrics_duration() {
    let start = Instant::now();
    let m = RunMetrics {
        start_time: Some(start),
        end_time: Some(start),
        ..Default::default()
    };
    // Same start and end should give 0
    assert_eq!(m.duration_ms(), 0);
}

#[test]
fn test_run_graph_state_new() {
    let options = make_options("hello");
    let state = RunGraphState::new("r1".into(), "s1".into(), "p1".into(), options);
    assert_eq!(state.run_id, "r1");
    assert_eq!(state.thread_id, "s1");
    assert_eq!(state.provider_key, "p1");
    assert_eq!(state.phase, RunPhase::Initialize);
    assert!(state.response.is_none());
    assert!(state.result.is_none());
    assert!(state.error.is_none());
}

#[test]
fn test_run_phase_equality() {
    assert_eq!(RunPhase::Initialize, RunPhase::Initialize);
    assert_ne!(RunPhase::Initialize, RunPhase::Execute);
    assert_ne!(RunPhase::Done, RunPhase::Cleanup);
}
