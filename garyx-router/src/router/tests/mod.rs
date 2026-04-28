use super::*;
use crate::memory_store::InMemoryThreadStore;
use crate::message_ledger::MessageLedgerStore;
use std::sync::Arc;

fn make_router() -> MessageRouter {
    let store = Arc::new(InMemoryThreadStore::new());
    let config = GaryxConfig::default();
    let mut router = MessageRouter::new(store, config);
    router.set_message_ledger_store(Arc::new(MessageLedgerStore::memory()));
    router
}

mod delivery;
mod dispatch;
mod inbound;
mod navigation;
mod routing;
