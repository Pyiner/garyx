//! Validated mutation witnesses for thread-record writes.
//!
//! Every merge-shaped write path takes one of these witnesses instead of a
//! raw JSON object, so the write contract is proven at construction time —
//! there is no unvalidated merge entry point on the store trait.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::ThreadStoreError;
use super::channel_bindings::{ChannelBindingsMergeAuthority, PROTECTED_CHANNEL_BINDINGS_FIELD};

/// One record's top-level field merge inside an atomic multi-record
/// mutation (see `ThreadStore::update_many_atomic`).
///
/// This is the privileged merge shape: it is the only write entry allowed
/// to touch the protected `channel_bindings` field, because moving an
/// endpoint binding is exactly the multi-record transition the atomic path
/// exists for. The privilege is structural, not a review rule: the fields
/// are private, [`Self::new`] rejects the protected field outright, and the
/// binding-carrying constructor [`Self::channel_bindings_merge`] demands a
/// borrow of the [`ChannelBindingsMergeAuthority`] witness, which has no
/// public constructor — it is provided by the `EndpointBindingMutator`
/// trait itself, plus a `test-seams`-gated fixture seam.
#[derive(Debug, Clone)]
pub struct AtomicRecordMerge {
    thread_id: String,
    fields: Value,
    /// Missing records normally abort the mutation; registry-style
    /// records owned by the mutation itself are created on first write.
    create_if_missing: bool,
}

impl AtomicRecordMerge {
    /// Unprivileged merge entry over ordinary top-level fields. The
    /// protected `channel_bindings` field is rejected here — exactly like
    /// [`ThreadRecordPatch`] — and non-object field payloads fail before
    /// storage is touched instead of silently merging nothing.
    pub fn new(
        thread_id: impl Into<String>,
        fields: Value,
        create_if_missing: bool,
    ) -> Result<Self, ThreadStoreError> {
        let thread_id = thread_id.into();
        let Some(object) = fields.as_object() else {
            return Err(ThreadStoreError::InvalidPatch(format!(
                "atomic merge fields for '{thread_id}' must be a JSON object"
            )));
        };
        if object.contains_key(PROTECTED_CHANNEL_BINDINGS_FIELD) {
            return Err(ThreadStoreError::InvalidPatch(format!(
                "protected field '{PROTECTED_CHANNEL_BINDINGS_FIELD}' requires the \
                 channel-bindings merge authority"
            )));
        }
        Ok(Self {
            thread_id,
            fields,
            create_if_missing,
        })
    }

    /// Privileged constructor for the endpoint-binding write path: merges
    /// the record snapshot's `channel_bindings` (missing means the empty
    /// list) plus its `updated_at` when present — exactly the two fields a
    /// binding move touches, never arbitrary payloads. Binding content
    /// comes from the caller's record snapshot; the authority witness
    /// proves the caller is an `EndpointBindingMutator` implementation
    /// (its trait-provided mint) or an explicit `test-seams` fixture.
    pub fn channel_bindings_merge(
        _authority: &ChannelBindingsMergeAuthority,
        thread_id: impl Into<String>,
        record: &Value,
        create_if_missing: bool,
    ) -> Self {
        let mut fields = Map::new();
        fields.insert(
            PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
            record
                .get(PROTECTED_CHANNEL_BINDINGS_FIELD)
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
        if let Some(updated_at) = record.get("updated_at") {
            fields.insert("updated_at".to_owned(), updated_at.clone());
        }
        Self {
            thread_id: thread_id.into(),
            fields: Value::Object(fields),
            create_if_missing,
        }
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn fields(&self) -> &Value {
        &self.fields
    }

    pub fn create_if_missing(&self) -> bool {
        self.create_if_missing
    }

    /// Decompose for backend application. Read-only consumers should use
    /// the borrowing accessors instead.
    pub fn into_parts(self) -> (String, Value, bool) {
        (self.thread_id, self.fields, self.create_if_missing)
    }
}

/// A validated top-level mutation for one existing thread record.
///
/// The fields are private so callers cannot accidentally carry an observed
/// whole body across an await. Use [`ThreadRecordPatch::from_diff`] to prove
/// that every changed field belongs to the caller's explicit allowlist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadRecordPatch {
    set_fields: Map<String, Value>,
    remove_fields: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadPatchResult {
    Applied,
    Unchanged,
}

impl ThreadRecordPatch {
    /// Build a patch from two complete observations while allowing only the
    /// named top-level fields to differ. Any unexpected difference is an
    /// error before storage is touched.
    pub fn from_diff(
        observed: &Value,
        desired: &Value,
        allowed_fields: &[&str],
    ) -> Result<Self, ThreadStoreError> {
        let observed = observed.as_object().ok_or_else(|| {
            ThreadStoreError::InvalidPatch("observed thread record is not an object".to_owned())
        })?;
        let desired = desired.as_object().ok_or_else(|| {
            ThreadStoreError::InvalidPatch("desired thread record is not an object".to_owned())
        })?;
        let allowed = allowed_fields.iter().copied().collect::<BTreeSet<_>>();
        let keys = observed
            .keys()
            .chain(desired.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut set_fields = Map::new();
        let mut remove_fields = BTreeSet::new();

        for key in keys {
            if observed.get(key) == desired.get(key) {
                continue;
            }
            if key == PROTECTED_CHANNEL_BINDINGS_FIELD {
                return Err(ThreadStoreError::InvalidPatch(format!(
                    "protected field '{key}' cannot be patched"
                )));
            }
            if !allowed.contains(key) {
                return Err(ThreadStoreError::InvalidPatch(format!(
                    "field '{key}' is outside the patch allowlist"
                )));
            }
            match desired.get(key) {
                Some(value) => {
                    set_fields.insert(key.to_owned(), value.clone());
                }
                None => {
                    remove_fields.insert(key.to_owned());
                }
            }
        }

        Self::validated(set_fields, remove_fields)
    }

    /// Build a patch from explicit fields. This is primarily useful for
    /// callers that did not start from a whole-record observation.
    pub fn new(
        set_fields: Map<String, Value>,
        remove_fields: BTreeSet<String>,
    ) -> Result<Self, ThreadStoreError> {
        Self::validated(set_fields, remove_fields)
    }

    fn validated(
        set_fields: Map<String, Value>,
        remove_fields: BTreeSet<String>,
    ) -> Result<Self, ThreadStoreError> {
        if set_fields.contains_key(PROTECTED_CHANNEL_BINDINGS_FIELD)
            || remove_fields.contains(PROTECTED_CHANNEL_BINDINGS_FIELD)
        {
            return Err(ThreadStoreError::InvalidPatch(format!(
                "protected field '{PROTECTED_CHANNEL_BINDINGS_FIELD}' cannot be patched"
            )));
        }
        if let Some(field) = set_fields.keys().find(|key| remove_fields.contains(*key)) {
            return Err(ThreadStoreError::InvalidPatch(format!(
                "field '{field}' cannot be both set and removed"
            )));
        }
        Ok(Self {
            set_fields,
            remove_fields,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.set_fields.is_empty() && self.remove_fields.is_empty()
    }

    pub fn changed_fields(&self) -> impl Iterator<Item = &str> {
        self.set_fields
            .keys()
            .map(String::as_str)
            .chain(self.remove_fields.iter().map(String::as_str))
    }

    pub fn apply_to(&self, record: &mut Value) -> Result<bool, ThreadStoreError> {
        let object = record.as_object_mut().ok_or_else(|| {
            ThreadStoreError::InvalidPatch("stored thread record is not an object".to_owned())
        })?;
        let mut changed = false;
        for (field, value) in &self.set_fields {
            if object.get(field) != Some(value) {
                object.insert(field.clone(), value.clone());
                changed = true;
            }
        }
        for field in &self.remove_fields {
            changed |= object.remove(field).is_some();
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn plain_merge_rejects_the_protected_channel_bindings_field() {
        let error = AtomicRecordMerge::new(
            "thread::plain",
            json!({"label": "ok", "channel_bindings": []}),
            false,
        )
        .expect_err("protected field must be rejected without the authority witness");
        assert!(matches!(error, ThreadStoreError::InvalidPatch(_)));
        assert!(error.to_string().contains(PROTECTED_CHANNEL_BINDINGS_FIELD));
    }

    #[test]
    fn plain_merge_rejects_non_object_fields() {
        let error = AtomicRecordMerge::new("thread::plain", json!(["not", "an", "object"]), false)
            .expect_err("non-object fields must fail before storage is touched");
        assert!(matches!(error, ThreadStoreError::InvalidPatch(_)));
    }

    #[test]
    fn plain_merge_accepts_ordinary_fields() {
        let merge = AtomicRecordMerge::new("thread::plain", json!({"label": "ok"}), true)
            .expect("ordinary fields are accepted");
        assert_eq!(merge.thread_id(), "thread::plain");
        assert!(merge.create_if_missing());
        let (thread_id, fields, create_if_missing) = merge.into_parts();
        assert_eq!(thread_id, "thread::plain");
        assert_eq!(fields, json!({"label": "ok"}));
        assert!(create_if_missing);
    }

    #[test]
    fn channel_bindings_merge_carries_exactly_bindings_and_updated_at() {
        let authority = ChannelBindingsMergeAuthority::test_authority();
        let record = json!({
            "thread_id": "thread::owner",
            "label": "never merged",
            "channel_bindings": [{"channel": "telegram"}],
            "updated_at": "2026-07-20T00:00:00Z"
        });
        let merge =
            AtomicRecordMerge::channel_bindings_merge(&authority, "thread::owner", &record, false);
        assert_eq!(
            merge.fields(),
            &json!({
                "channel_bindings": [{"channel": "telegram"}],
                "updated_at": "2026-07-20T00:00:00Z"
            }),
            "only the protected field and updated_at may travel in a binding merge"
        );
        assert!(!merge.create_if_missing());
    }

    #[test]
    fn channel_bindings_merge_defaults_missing_bindings_to_the_empty_list() {
        let authority = ChannelBindingsMergeAuthority::test_authority();
        let merge = AtomicRecordMerge::channel_bindings_merge(
            &authority,
            "meta::known_channel_endpoints",
            &json!({"label": "no bindings, no updated_at"}),
            true,
        );
        assert_eq!(merge.fields(), &json!({"channel_bindings": []}));
        assert!(merge.create_if_missing());
    }
}
