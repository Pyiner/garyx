//! The built-in outbound sender registry — the ONLY place in the
//! crate that can recover a concrete sender type from the erased
//! collection.
//!
//! Structural containment (Phase-6 B2 review): the downcast
//! capability lives on a PRIVATE trait ([`ErasedSender`]) implemented
//! via a blanket impl, and the boxed senders live in a PRIVATE field.
//! Outside this module the only surfaces are:
//!
//! - [`BuiltinSenderRegistry::route`] / [`BuiltinSenderRegistry::iter`]
//!   — type-erased, `&self`, suitable for dispatch paths;
//! - [`BuiltinSenderRegistry::with_mut`] — typed access for the
//!   registration/construction layer, deliberately `&mut self` so it
//!   is inexpressible inside `&self` routing contexts.
//!
//! [`crate::dispatcher::OutboundChannelSender`] itself carries no
//! `Any`/downcast/clone-box surface, so holding a
//! `&dyn OutboundChannelSender` (what `route` returns) cannot be
//! turned back into a concrete channel type. A channel special-case
//! in the dispatcher would need either a channel-name string literal
//! (caught by the literal guard) or this module's private trait
//! (privacy error).

use std::any::Any;

use crate::dispatcher::OutboundChannelSender;

/// Private erasure shim: adds downcast + clone-box on top of the
/// public sender contract. Implemented for every concrete sender via
/// the blanket impl below; never nameable outside this module.
trait ErasedSender: OutboundChannelSender {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn ErasedSender>;
}

impl<T> ErasedSender for T
where
    T: OutboundChannelSender + Clone + 'static,
{
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ErasedSender> {
        Box::new(self.clone())
    }
}

/// Ordered collection of the built-in channel senders. Construction
/// happens in [`crate::builtin_catalog::builtin_sender_registry`];
/// the dispatcher receives the finished registry and can only route
/// through it.
pub(crate) struct BuiltinSenderRegistry {
    senders: Vec<Box<dyn ErasedSender>>,
}

impl BuiltinSenderRegistry {
    pub(crate) fn new() -> Self {
        Self {
            senders: Vec::new(),
        }
    }

    /// Register one concrete built-in sender (construction layer).
    pub(crate) fn push<T>(&mut self, sender: T)
    where
        T: OutboundChannelSender + Clone + 'static,
    {
        self.senders.push(Box::new(sender));
    }

    /// Resolve a channel name (canonical id or alias) to its sender.
    pub(crate) fn route(&self, name: &str) -> Option<&dyn OutboundChannelSender> {
        self.iter()
            .find(|sender| sender.channel_id() == name || sender.aliases().contains(&name))
    }

    /// Iterate the senders type-erased (member listing).
    pub(crate) fn iter(&self) -> impl Iterator<Item = &dyn OutboundChannelSender> {
        self.senders
            .iter()
            .map(|sender| sender.as_ref() as &dyn OutboundChannelSender)
    }

    /// Typed access for registration/config seeding. `&mut self` on
    /// purpose: `&self` dispatch paths cannot express this. Panics
    /// only on a construction bug (catalog did not register `T`).
    pub(crate) fn with_mut<T: 'static, R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        let sender = self
            .senders
            .iter_mut()
            .find_map(|sender| sender.as_any_mut().downcast_mut::<T>())
            .expect("builtin sender registered at construction");
        f(sender)
    }
}

impl Clone for BuiltinSenderRegistry {
    fn clone(&self) -> Self {
        Self {
            senders: self
                .senders
                .iter()
                .map(|sender| sender.clone_box())
                .collect(),
        }
    }
}
