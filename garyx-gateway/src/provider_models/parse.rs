use super::*;

pub(super) fn parse_model_array(values: &[Value]) -> Vec<ProviderModelOption> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for value in values {
        let Some(option) = parse_model_option(value) else {
            continue;
        };
        if seen.insert(option.id.clone()) {
            models.push(option);
        }
    }
    models
}

pub(super) fn parse_model_option(value: &Value) -> Option<ProviderModelOption> {
    if let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) {
        return Some(ProviderModelOption {
            id: id.to_owned(),
            label: model_label(id, None),
            description: None,
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        });
    }

    let object = value.as_object()?;
    let id = ["id", "name", "model", "model_id", "modelId"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let label = ["label", "display_name", "displayName", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|label| !label.is_empty());
    let description = ["description", "summary"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned);

    Some(ProviderModelOption {
        id: id.to_owned(),
        label: model_label(id, label),
        description,
        recommended: object
            .get("recommended")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
    })
}

pub(super) fn model_label(id: &str, label: Option<&str>) -> String {
    label
        .map(str::to_owned)
        .unwrap_or_else(|| id.strip_prefix("models/").unwrap_or(id).to_owned())
}

pub(super) fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
