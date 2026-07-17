use serde_json::Value;

/// Remove the retired recent-membership exclusion flags from a canonical
/// thread record. This invariant is shared by runtime writes and every
/// versioned cleanup that can encounter legacy/imported bodies.
pub(crate) fn strip_retired_recent_exclusion_fields(data: &mut Value) -> bool {
    let Some(object) = data.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    for key in ["exclude_from_recent", "excludeFromRecent"] {
        changed |= object.remove(key).is_some();
    }
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        for key in ["exclude_from_recent", "excludeFromRecent"] {
            changed |= metadata.remove(key).is_some();
        }
    }
    changed
}
