use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use crate::client::ClaudeSDKClient;
use crate::error::{ClaudeSDKError, Result};
use crate::types::{ClaudeAgentOptions, McpServerConfig, Message, ResultMessage};

/// User input payload sent to a running streaming task.
#[derive(Debug, Clone, PartialEq)]
pub enum UserInput {
    Text(String),
    Blocks(Vec<Value>),
}

/// Typed outbound user message for streaming sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundUserMessage {
    pub content: UserInput,
    pub session_id: String,
    pub parent_tool_use_id: Option<String>,
}

impl OutboundUserMessage {
    pub fn text(text: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            content: UserInput::Text(text.into()),
            session_id: session_id.into(),
            parent_tool_use_id: None,
        }
    }

    pub fn blocks(blocks: Vec<Value>, session_id: impl Into<String>) -> Self {
        Self {
            content: UserInput::Blocks(blocks),
            session_id: session_id.into(),
            parent_tool_use_id: None,
        }
    }
}

struct RunState {
    client: Mutex<ClaudeSDKClient>,
    closed: AtomicBool,
}

/// Cloneable control handle for a live Claude streaming task.
#[derive(Clone)]
pub struct ClaudeRunControl {
    state: Arc<RunState>,
}

impl ClaudeRunControl {
    fn ensure_open(&self) -> Result<()> {
        if self.state.closed.load(Ordering::SeqCst) {
            Err(ClaudeSDKError::Control(
                "streaming run is already closed".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    pub async fn send_user_message(&self, msg: OutboundUserMessage) -> Result<()> {
        self.ensure_open()?;
        let OutboundUserMessage {
            content,
            session_id,
            parent_tool_use_id,
        } = msg;

        let content = match content {
            UserInput::Text(text) => Value::String(text),
            UserInput::Blocks(blocks) => Value::Array(blocks),
        };
        let session_id = (!session_id.is_empty()).then_some(session_id.as_str());

        let guard = self.state.client.lock().await;
        guard
            .send_user_content(content, session_id, parent_tool_use_id.as_deref())
            .await
    }

    pub async fn interrupt(&self) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.interrupt().await
    }

    pub async fn set_permission_mode(&self, mode: &str) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.set_permission_mode(mode).await
    }

    pub async fn set_model(&self, model: Option<&str>) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.set_model(model).await
    }

    pub async fn set_max_thinking_tokens(&self, max_thinking_tokens: Option<i64>) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.set_max_thinking_tokens(max_thinking_tokens).await
    }

    pub async fn mcp_server_status(&self) -> Result<Value> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.get_mcp_status().await
    }

    pub async fn set_mcp_servers(
        &self,
        servers: HashMap<String, McpServerConfig>,
    ) -> Result<Value> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.set_mcp_servers(servers).await
    }

    pub async fn reconnect_mcp_server(&self, server_name: &str) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.reconnect_mcp_server(server_name).await
    }

    pub async fn toggle_mcp_server(&self, server_name: &str, enabled: bool) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.toggle_mcp_server(server_name, enabled).await
    }

    pub async fn rewind_files(&self, user_message_id: &str) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.rewind_files(user_message_id).await
    }

    pub async fn rewind_files_dry_run(&self, user_message_id: &str) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.rewind_files_dry_run(user_message_id).await
    }

    pub async fn stop_task(&self, task_id: &str) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.stop_task(task_id).await
    }

    pub async fn apply_flag_settings(&self, settings: Value) -> Result<()> {
        self.ensure_open()?;
        let guard = self.state.client.lock().await;
        guard.apply_flag_settings(settings).await
    }

    pub async fn close(&self) -> Result<()> {
        if self.state.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let mut guard = self.state.client.lock().await;
        guard.disconnect().await
    }
}

/// Running streaming task handle.
pub struct ClaudeRun {
    messages: mpsc::Receiver<Result<Message>>,
    control: ClaudeRunControl,
}

impl ClaudeRun {
    pub fn control(&self) -> ClaudeRunControl {
        self.control.clone()
    }

    pub async fn next_message(&mut self) -> Option<Result<Message>> {
        self.messages.recv().await
    }

    pub async fn collect_until_result(&mut self) -> Result<ResultMessage> {
        while let Some(message) = self.next_message().await {
            match message {
                Ok(Message::Result(result)) => return Ok(result),
                Ok(_) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(ClaudeSDKError::Connection(
            "stream ended without a result message".to_owned(),
        ))
    }

    pub async fn close(&self) -> Result<()> {
        self.control.close().await
    }
}

/// Start a Claude task in streaming mode and return a run handle.
pub async fn run_streaming(options: ClaudeAgentOptions) -> Result<ClaudeRun> {
    let mut client = ClaudeSDKClient::new(options);
    client.connect(None).await?;

    let messages = client.take_message_receiver().ok_or_else(|| {
        ClaudeSDKError::Connection("failed to acquire message receiver".to_owned())
    })?;

    let state = Arc::new(RunState {
        client: Mutex::new(client),
        closed: AtomicBool::new(false),
    });

    Ok(ClaudeRun {
        messages,
        control: ClaudeRunControl { state },
    })
}

#[cfg(test)]
mod tests;
