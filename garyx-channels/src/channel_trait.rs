use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("connection error: {0}")]
    Connection(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&mut self) -> Result<(), ChannelError>;
    async fn stop(&mut self) -> Result<(), ChannelError>;
    fn is_running(&self) -> bool;
}
