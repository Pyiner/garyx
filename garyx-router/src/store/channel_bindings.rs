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

/// Capability witness for constructing binding-carrying
/// [`AtomicRecordMerge`](super::AtomicRecordMerge) entries.
///
/// The repository contract (docs/agents/repository-contracts.md) makes
/// `update_many_atomic` the only write shape allowed to change
/// `channel_bindings`, and the serialized `EndpointBindingMutator` service
/// the only driver of that shape. This witness turns the second half of
/// that rule structural: `AtomicRecordMerge::new` rejects the protected
/// field outright, and the binding-carrying constructor demands a borrow
/// of this authority.
///
/// There is deliberately NO public constructor. The only sources are:
/// - the provided `EndpointBindingMutator::binding_merge_authority`
///   method — implementing that trait is the declaration of being the
///   serialized binding mutator, so the capability rides on the trait
///   itself and cannot be minted by ordinary `ThreadStore` callers; and
/// - [`Self::test_authority`], a `test-seams`-gated seam for fixtures
///   that inject binding state without being a mutator.
#[derive(Debug)]
pub struct ChannelBindingsMergeAuthority {
    _witness: (),
}

impl ChannelBindingsMergeAuthority {
    /// Crate-internal mint backing the provided
    /// `EndpointBindingMutator::binding_merge_authority` method. Not
    /// callable outside `garyx-router`; downstream implementors inherit
    /// the provided method body instead of minting themselves.
    pub(crate) fn mutator_provided() -> Self {
        Self { _witness: () }
    }

    /// Explicit test-only seam: mint an authority for a fixture that
    /// injects binding state (registry drift, simulated concurrent moves)
    /// without being an endpoint-binding mutator. Enabled only for
    /// in-crate tests and downstream `[dev-dependencies]` that opt into
    /// the `test-seams` feature; production builds have no such entry.
    #[cfg(any(test, feature = "test-seams"))]
    pub fn test_authority() -> Self {
        Self { _witness: () }
    }
}

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
