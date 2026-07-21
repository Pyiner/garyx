//! Guards for the protected `channel_bindings` record field.
//!
//! Endpoint-binding ownership is mutated only through the privileged
//! multi-record path (`ThreadStore::update_many_atomic`, driven by the
//! gateway's `EndpointBindingMutator`) and thread creation. Every other
//! write shape must leave the field untouched; these helpers are the
//! shared checks backends run inside their own write locks.

use std::collections::BTreeMap;

use serde_json::Value;

use super::ThreadStoreError;
use crate::threads::ChannelBinding;

pub(super) const PROTECTED_CHANNEL_BINDINGS_FIELD: &str = "channel_bindings";

fn canonical_channel_bindings(
    thread_id: &str,
    record: &Value,
) -> Result<BTreeMap<String, ChannelBinding>, ThreadStoreError> {
    let Some(value) = record.get(PROTECTED_CHANNEL_BINDINGS_FIELD) else {
        return Ok(BTreeMap::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| ThreadStoreError::ProtectedFieldConflict {
            thread_id: thread_id.to_owned(),
            field: PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
        })?;
    let mut bindings = BTreeMap::new();
    for item in items {
        let binding = serde_json::from_value::<ChannelBinding>(item.clone()).map_err(|_| {
            ThreadStoreError::ProtectedFieldConflict {
                thread_id: thread_id.to_owned(),
                field: PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
            }
        })?;
        if binding.channel.trim().is_empty()
            || binding.account_id.trim().is_empty()
            || binding.binding_key.trim().is_empty()
        {
            return Err(ThreadStoreError::ProtectedFieldConflict {
                thread_id: thread_id.to_owned(),
                field: PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
            });
        }
        let endpoint_key = binding.endpoint_key();
        if bindings.insert(endpoint_key, binding).is_some() {
            return Err(ThreadStoreError::ProtectedFieldConflict {
                thread_id: thread_id.to_owned(),
                field: PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
            });
        }
    }
    Ok(bindings)
}

pub fn validate_channel_bindings(thread_id: &str, record: &Value) -> Result<(), ThreadStoreError> {
    canonical_channel_bindings(thread_id, record).map(|_| ())
}

/// Reject an existing-record replacement that changes endpoint ownership or
/// binding metadata. Missing and an empty list are semantically equivalent.
pub fn ensure_channel_bindings_unchanged(
    thread_id: &str,
    current: &Value,
    incoming: &Value,
) -> Result<(), ThreadStoreError> {
    if canonical_channel_bindings(thread_id, current)?
        == canonical_channel_bindings(thread_id, incoming)?
    {
        Ok(())
    } else {
        Err(ThreadStoreError::ProtectedFieldConflict {
            thread_id: thread_id.to_owned(),
            field: PROTECTED_CHANNEL_BINDINGS_FIELD.to_owned(),
        })
    }
}
