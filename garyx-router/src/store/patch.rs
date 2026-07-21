//! Validated mutation witnesses for thread-record writes.
//!
//! Every merge-shaped write path takes one of these witnesses instead of a
//! raw JSON object, so the write contract is proven at construction time —
//! there is no unvalidated merge entry point on the store trait.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::ThreadStoreError;
use super::channel_bindings::PROTECTED_CHANNEL_BINDINGS_FIELD;

/// One record's top-level field merge inside an atomic multi-record
/// mutation (see `ThreadStore::update_many_atomic`).
///
/// This is the privileged merge shape: it is the only write entry allowed
/// to touch the protected `channel_bindings` field, because moving an
/// endpoint binding is exactly the multi-record transition the atomic path
/// exists for. Callers outside the endpoint-binding mutator and atomic
/// thread creation must not put protected fields in `fields`.
#[derive(Debug, Clone)]
pub struct AtomicRecordMerge {
    pub thread_id: String,
    pub fields: Value,
    /// Missing records normally abort the mutation; registry-style
    /// records owned by the mutation itself are created on first write.
    pub create_if_missing: bool,
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
