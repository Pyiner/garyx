use garyx_router::thread_kind_from_value;
use serde_json::Value;

pub(crate) fn thread_summary_type_from_record(data: &Value) -> String {
    thread_kind_from_value(data).unwrap_or_else(|| "chat".to_owned())
}
