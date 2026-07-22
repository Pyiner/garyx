use serde_json::Value;

/// The write-time preview fields projected from one canonical thread record.
///
/// Both list projections consume this value so `last_message_preview` has one
/// gateway-owned meaning: the newest user preview when present, otherwise the
/// newest assistant preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadMessagePreviews {
    pub(crate) last_user: Option<String>,
    pub(crate) last_assistant: Option<String>,
}

impl ThreadMessagePreviews {
    pub(crate) fn user_first(&self) -> Option<String> {
        self.last_user
            .clone()
            .or_else(|| self.last_assistant.clone())
    }
}

pub(crate) fn thread_message_previews(data: &Value) -> ThreadMessagePreviews {
    ThreadMessagePreviews {
        last_user: preview_for_role(data, "user"),
        last_assistant: preview_for_role(data, "assistant"),
    }
}

fn preview_for_role(data: &Value, role: &str) -> Option<String> {
    garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|preview| !preview.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn shared_projection_is_user_first_for_every_gateway_consumer() {
        let previews = thread_message_previews(&json!({
            "last_user_preview": "Latest user sentence",
            "last_assistant_preview": "Assistant answer",
        }));

        assert_eq!(previews.last_user.as_deref(), Some("Latest user sentence"));
        assert_eq!(previews.last_assistant.as_deref(), Some("Assistant answer"));
        assert_eq!(
            previews.user_first().as_deref(),
            Some("Latest user sentence")
        );
    }

    #[test]
    fn shared_projection_falls_back_to_non_blank_assistant_preview() {
        let previews = thread_message_previews(&json!({
            "last_user_preview": "   ",
            "last_assistant_preview": "Assistant only",
        }));

        assert_eq!(previews.last_user, None);
        assert_eq!(previews.user_first().as_deref(), Some("Assistant only"));
    }
}
