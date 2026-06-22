use super::*;

pub(super) fn validate_json_size(
    field: &str,
    value: &Value,
    max_bytes: usize,
) -> Result<(), WorkflowError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        WorkflowError::BadRequest(format!("{field} must be JSON-serializable: {error}"))
    })?;
    if bytes.len() > max_bytes {
        return Err(WorkflowError::BadRequest(format!(
            "{field} exceeds {} bytes",
            max_bytes
        )));
    }
    Ok(())
}

pub(super) fn validate_schema_shape(schema: &Value, depth: usize) -> Result<(), WorkflowError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(WorkflowError::BadRequest(
            "schema nesting is too deep".to_owned(),
        ));
    }
    let Some(object) = schema.as_object() else {
        return Err(WorkflowError::BadRequest(
            "schema must be a JSON object".to_owned(),
        ));
    };
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkflowError::BadRequest("schema.type is required".to_owned()))?;
    match schema_type {
        "object" => {
            if let Some(properties) = object.get("properties") {
                let Some(properties) = properties.as_object() else {
                    return Err(WorkflowError::BadRequest(
                        "schema.properties must be an object".to_owned(),
                    ));
                };
                for value in properties.values() {
                    validate_schema_shape(value, depth + 1)?;
                }
            }
        }
        "array" => {
            if let Some(items) = object.get("items") {
                validate_schema_shape(items, depth + 1)?;
            }
        }
        "string" | "number" | "integer" | "boolean" | "null" => {}
        other => {
            return Err(WorkflowError::BadRequest(format!(
                "unsupported schema type: {other}"
            )));
        }
    }
    if let Some(enum_values) = object.get("enum")
        && !enum_values.is_array()
    {
        return Err(WorkflowError::BadRequest(
            "schema.enum must be an array".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_payload_against_schema(
    schema: &Value,
    payload: &Value,
    path: &str,
) -> Result<(), WorkflowError> {
    let object = schema
        .as_object()
        .ok_or_else(|| WorkflowError::BadRequest(format!("{path}: schema must be object")))?;
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkflowError::BadRequest(format!("{path}: schema.type is required")))?;
    match schema_type {
        "object" => {
            let Some(payload_object) = payload.as_object() else {
                return Err(WorkflowError::BadRequest(format!(
                    "{path}: expected object"
                )));
            };
            if let Some(required) = object.get("required").and_then(Value::as_array) {
                for field in required.iter().filter_map(Value::as_str) {
                    if !payload_object.contains_key(field) {
                        return Err(WorkflowError::BadRequest(format!(
                            "{path}: missing required field {field}"
                        )));
                    }
                }
            }
            let properties = object
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if object.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
                for key in payload_object.keys() {
                    if !properties.contains_key(key) {
                        return Err(WorkflowError::BadRequest(format!(
                            "{path}: additional property {key} is not allowed"
                        )));
                    }
                }
            }
            for (key, child_schema) in properties {
                if let Some(child_payload) = payload_object.get(&key) {
                    validate_payload_against_schema(
                        &child_schema,
                        child_payload,
                        &format!("{path}.{key}"),
                    )?;
                }
            }
        }
        "array" => {
            let Some(items) = payload.as_array() else {
                return Err(WorkflowError::BadRequest(format!("{path}: expected array")));
            };
            if let Some(item_schema) = object.get("items") {
                for (index, item) in items.iter().enumerate() {
                    validate_payload_against_schema(
                        item_schema,
                        item,
                        &format!("{path}[{index}]"),
                    )?;
                }
            }
        }
        "string" if !payload.is_string() => {
            return Err(WorkflowError::BadRequest(format!(
                "{path}: expected string"
            )));
        }
        "number" if !payload.is_number() => {
            return Err(WorkflowError::BadRequest(format!(
                "{path}: expected number"
            )));
        }
        "integer" if payload.as_i64().is_none() && payload.as_u64().is_none() => {
            return Err(WorkflowError::BadRequest(format!(
                "{path}: expected integer"
            )));
        }
        "boolean" if !payload.is_boolean() => {
            return Err(WorkflowError::BadRequest(format!(
                "{path}: expected boolean"
            )));
        }
        "null" if !payload.is_null() => {
            return Err(WorkflowError::BadRequest(format!("{path}: expected null")));
        }
        "string" | "number" | "integer" | "boolean" | "null" => {}
        other => {
            return Err(WorkflowError::BadRequest(format!(
                "{path}: unsupported schema type {other}"
            )));
        }
    }
    if let Some(enum_values) = object.get("enum").and_then(Value::as_array)
        && !enum_values.iter().any(|candidate| candidate == payload)
    {
        return Err(WorkflowError::BadRequest(format!(
            "{path}: value is not allowed by enum"
        )));
    }
    Ok(())
}

pub(super) fn normalize_submitted_payload(schema: &Value, payload: Value) -> Value {
    let Value::String(raw) = &payload else {
        return payload;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return payload;
    };
    if validate_payload_against_schema(schema, &parsed, "$").is_ok() {
        parsed
    } else {
        payload
    }
}
