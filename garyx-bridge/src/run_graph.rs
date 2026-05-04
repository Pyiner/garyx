use std::sync::{Arc, Mutex};
use std::time::Instant;

use garyx_models::agent::RunState;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, StreamEvent};

use crate::graph_engine::{Graph, GraphBuildError, GraphError, GraphTransition, NodeFuture};
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

// ---------------------------------------------------------------------------
// RunPhase — state machine phases
// ---------------------------------------------------------------------------

/// Phases of the agent run execution graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunPhase {
    Initialize,
    SetupProgress,
    Execute,
    ProcessResult,
    Cleanup,
    Done,
}

// ---------------------------------------------------------------------------
// RunMetrics
// ---------------------------------------------------------------------------

/// Metrics collected during an agent run.
#[derive(Debug, Clone)]
pub struct RunMetrics {
    pub state: RunState,
    pub start_time: Option<Instant>,
    pub first_token_time: Option<Instant>,
    pub end_time: Option<Instant>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub error_message: Option<String>,
}

impl Default for RunMetrics {
    fn default() -> Self {
        Self {
            state: RunState::Pending,
            start_time: None,
            first_token_time: None,
            end_time: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            error_message: None,
        }
    }
}

impl RunMetrics {
    /// Duration from start to end (or to now if still running).
    pub fn duration_ms(&self) -> i64 {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => end.duration_since(start).as_millis() as i64,
            (Some(start), None) => start.elapsed().as_millis() as i64,
            _ => 0,
        }
    }

    /// Time from start to first token.
    pub fn time_to_first_token_ms(&self) -> i64 {
        match (self.start_time, self.first_token_time) {
            (Some(start), Some(ftt)) => ftt.duration_since(start).as_millis() as i64,
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// RunGraphState
// ---------------------------------------------------------------------------

/// Mutable state container for a single agent run execution.
pub struct RunGraphState {
    pub run_id: String,
    pub thread_id: String,
    pub provider_key: String,
    pub run_options: ProviderRunOptions,
    pub phase: RunPhase,
    pub metrics: RunMetrics,
    pub response: Option<String>,
    pub result: Option<ProviderRunResult>,
    pub error: Option<String>,
}

impl RunGraphState {
    /// Create a new state for an agent run.
    pub fn new(
        run_id: String,
        thread_id: String,
        provider_key: String,
        run_options: ProviderRunOptions,
    ) -> Self {
        Self {
            run_id,
            thread_id,
            provider_key,
            run_options,
            phase: RunPhase::Initialize,
            metrics: RunMetrics::default(),
            response: None,
            result: None,
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal graph execution context
// ---------------------------------------------------------------------------

type RunGraphOutput = Result<ProviderRunResult, BridgeError>;

struct RunGraphExecution<'a> {
    provider: &'a dyn AgentLoopProvider,
    state: &'a mut RunGraphState,
    response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    exec_result: Option<Result<ProviderRunResult, BridgeError>>,
    terminal_result: Option<RunGraphOutput>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum RunGraphNode {
    Initialize,
    SetupProgress,
    Execute,
    ProcessResult,
    Cleanup,
}

fn node_initialize<'ctx, 'deps>(
    ctx: &'ctx mut RunGraphExecution<'deps>,
) -> NodeFuture<'ctx, RunGraphNode, RunGraphOutput, BridgeError> {
    Box::pin(async move {
        let state = &mut ctx.state;
        state.phase = RunPhase::Initialize;

        if !ctx.provider.is_ready() {
            state.metrics.state = RunState::Error;
            state.metrics.error_message = Some("provider not ready".to_owned());
            state.error = Some("provider not ready".to_owned());
            ctx.terminal_result = Some(Err(BridgeError::ProviderNotReady));
            return Ok(GraphTransition::Next(RunGraphNode::Cleanup));
        }

        state.metrics = RunMetrics::default();
        state.metrics.state = RunState::Pending;

        tracing::info!(
            run_id = %state.run_id,
            thread_id = %state.thread_id,
            provider_key = %state.provider_key,
            "run graph: initialized"
        );

        Ok(GraphTransition::Next(RunGraphNode::SetupProgress))
    })
}

fn node_setup_progress<'ctx, 'deps>(
    ctx: &'ctx mut RunGraphExecution<'deps>,
) -> NodeFuture<'ctx, RunGraphNode, RunGraphOutput, BridgeError> {
    Box::pin(async move {
        ctx.state.phase = RunPhase::SetupProgress;

        // Progress heartbeat is a future enhancement; for now this is a no-op
        // that keeps the phase structure consistent with the Python graph.
        tracing::debug!(
            run_id = %ctx.state.run_id,
            "run graph: progress setup complete"
        );

        Ok(GraphTransition::Next(RunGraphNode::Execute))
    })
}

fn node_execute<'ctx, 'deps>(
    ctx: &'ctx mut RunGraphExecution<'deps>,
) -> NodeFuture<'ctx, RunGraphNode, RunGraphOutput, BridgeError> {
    Box::pin(async move {
        // An earlier node may already have decided a terminal outcome.
        if ctx.terminal_result.is_some() {
            return Ok(GraphTransition::Next(RunGraphNode::Cleanup));
        }

        let state = &mut ctx.state;
        state.phase = RunPhase::Execute;
        state.metrics.state = RunState::Running;
        state.metrics.start_time = Some(Instant::now());

        let exec_result = if let Some(cb) = ctx.response_callback.clone() {
            let first_token_instant = Arc::new(Mutex::new(None::<Instant>));
            let first_token_instant_cb = Arc::clone(&first_token_instant);

            let stream_cb: StreamCallback = Box::new(move |event: StreamEvent| {
                if let StreamEvent::Delta { text } = &event
                    && !text.is_empty()
                {
                    if let Ok(mut first_token) = first_token_instant_cb.lock() {
                        if first_token.is_none() {
                            *first_token = Some(Instant::now());
                        }
                    } else {
                        tracing::warn!("failed to record first token timestamp: mutex poisoned");
                    }
                }
                cb(event);
            });

            let result = ctx
                .provider
                .run_streaming(&state.run_options, stream_cb)
                .await;

            if state.metrics.first_token_time.is_none() {
                if let Ok(captured) = first_token_instant.lock() {
                    if let Some(ftt) = *captured {
                        state.metrics.first_token_time = Some(ftt);
                        state.metrics.state = RunState::Streaming;
                    }
                } else {
                    tracing::warn!("failed to finalize first token timestamp: mutex poisoned");
                }
            }

            result
        } else {
            let noop: StreamCallback = Box::new(|_| {});
            ctx.provider.run_streaming(&state.run_options, noop).await
        };

        ctx.exec_result = Some(exec_result);
        Ok(GraphTransition::Next(RunGraphNode::ProcessResult))
    })
}

fn node_process_result<'ctx, 'deps>(
    ctx: &'ctx mut RunGraphExecution<'deps>,
) -> NodeFuture<'ctx, RunGraphNode, RunGraphOutput, BridgeError> {
    Box::pin(async move {
        ctx.state.phase = RunPhase::ProcessResult;

        if ctx.terminal_result.is_some() {
            return Ok(GraphTransition::Next(RunGraphNode::Cleanup));
        }

        match ctx.exec_result.take() {
            Some(Ok(result)) => {
                ctx.state.response = Some(result.response.clone());
                ctx.state.metrics.input_tokens = result.input_tokens as u64;
                ctx.state.metrics.output_tokens = result.output_tokens as u64;
                ctx.state.metrics.cost_usd = result.cost;
                ctx.state.metrics.state = RunState::Completed;
                ctx.state.metrics.end_time = Some(Instant::now());

                tracing::info!(
                    run_id = %ctx.state.run_id,
                    success = result.success,
                    cost_usd = result.cost,
                    input_tokens = result.input_tokens,
                    output_tokens = result.output_tokens,
                    duration_ms = ctx.state.metrics.duration_ms(),
                    "run graph: execution completed"
                );

                ctx.state.result = Some(result.clone());
                ctx.terminal_result = Some(Ok(result));
            }
            Some(Err(e)) => {
                ctx.state.metrics.state = RunState::Error;
                ctx.state.metrics.error_message = Some(e.to_string());
                ctx.state.metrics.end_time = Some(Instant::now());
                ctx.state.error = Some(e.to_string());

                tracing::error!(
                    run_id = %ctx.state.run_id,
                    error = %e,
                    duration_ms = ctx.state.metrics.duration_ms(),
                    "run graph: execution failed"
                );

                ctx.terminal_result = Some(Err(e));
            }
            None => {
                let e = BridgeError::Internal("missing execution result in run graph".to_owned());
                ctx.state.metrics.state = RunState::Error;
                ctx.state.metrics.error_message = Some(e.to_string());
                ctx.state.metrics.end_time = Some(Instant::now());
                ctx.state.error = Some(e.to_string());
                ctx.terminal_result = Some(Err(e));
            }
        }

        Ok(GraphTransition::Next(RunGraphNode::Cleanup))
    })
}

fn node_cleanup<'ctx, 'deps>(
    ctx: &'ctx mut RunGraphExecution<'deps>,
) -> NodeFuture<'ctx, RunGraphNode, RunGraphOutput, BridgeError> {
    Box::pin(async move {
        ctx.state.phase = RunPhase::Cleanup;

        tracing::debug!(
            run_id = %ctx.state.run_id,
            "run graph: cleanup complete"
        );

        ctx.state.phase = RunPhase::Done;
        let output = ctx.terminal_result.take().unwrap_or_else(|| {
            Err(BridgeError::Internal(
                "run graph produced no terminal result".to_owned(),
            ))
        });
        Ok(GraphTransition::End(output))
    })
}

fn build_agent_run_graph<'a>() -> Result<
    Graph<RunGraphExecution<'a>, RunGraphNode, RunGraphOutput, BridgeError>,
    GraphBuildError<RunGraphNode>,
> {
    let mut graph = Graph::new(RunGraphNode::Initialize).with_max_steps(16);
    graph.add_node(RunGraphNode::Initialize, node_initialize)?;
    graph.add_node(RunGraphNode::SetupProgress, node_setup_progress)?;
    graph.add_node(RunGraphNode::Execute, node_execute)?;
    graph.add_node(RunGraphNode::ProcessResult, node_process_result)?;
    graph.add_node(RunGraphNode::Cleanup, node_cleanup)?;
    graph.validate()?;
    Ok(graph)
}

fn map_graph_build_error(err: GraphBuildError<RunGraphNode>) -> BridgeError {
    match err {
        GraphBuildError::DuplicateNode(node) => {
            BridgeError::Internal(format!("invalid run graph: duplicate node '{node:?}'"))
        }
        GraphBuildError::MissingEntryNode(node) => BridgeError::Internal(format!(
            "invalid run graph: entry node '{node:?}' is not registered"
        )),
        GraphBuildError::EmptyGraph => {
            BridgeError::Internal("invalid run graph: no nodes registered".to_owned())
        }
    }
}

fn map_graph_runtime_error(err: GraphError<RunGraphNode, BridgeError>) -> BridgeError {
    match err {
        GraphError::Node(e) => e,
        GraphError::NodeNotFound(node) => {
            BridgeError::Internal(format!("invalid run graph: node '{node:?}' not found"))
        }
        GraphError::StepLimitExceeded(limit) => {
            BridgeError::Internal(format!("run graph exceeded step limit ({limit})"))
        }
    }
}

// ---------------------------------------------------------------------------
// execute_agent_run — graph runtime entrypoint
// ---------------------------------------------------------------------------

/// Execute an agent run through the run graph.
pub async fn execute_agent_run(
    provider: &dyn AgentLoopProvider,
    state: &mut RunGraphState,
    response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
) -> Result<ProviderRunResult, BridgeError> {
    let graph = build_agent_run_graph().map_err(map_graph_build_error)?;

    let mut execution = RunGraphExecution {
        provider,
        state,
        response_callback,
        exec_result: None,
        terminal_result: None,
    };

    let result = graph
        .run(&mut execution)
        .await
        .map_err(map_graph_runtime_error)?;

    result.output
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
