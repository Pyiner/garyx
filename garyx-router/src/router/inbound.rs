use serde_json::Value;

use super::{InboundRequest, NATIVE_COMMAND_TEXT_METADATA_KEY};
use crate::router::command_catalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeThreadCommand {
    Threads,
    New,
    ThreadPrev,
    ThreadNext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeCommand {
    Thread(NativeThreadCommand),
    Loop,
}

pub(super) struct InboundCommandClassifier;

impl InboundCommandClassifier {
    pub(super) fn parse(text: &str, channel: &str) -> Option<NativeCommand> {
        command_catalog::native_command_from_text(text, channel)
    }

    pub(super) fn command_text(request: &InboundRequest) -> Option<&str> {
        request
            .extra_metadata
            .get(NATIVE_COMMAND_TEXT_METADATA_KEY)
            .and_then(Value::as_str)
    }

    pub(super) fn name(command: NativeCommand) -> &'static str {
        command_catalog::native_command_name(command)
    }
}

pub fn is_native_command_text(text: &str, channel: &str) -> bool {
    InboundCommandClassifier::parse(text, channel).is_some()
}
