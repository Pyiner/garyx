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
//! - [`BuiltinSenderRegistry::register`] — the consume-only sealed
//!   registration entry: no caller closure, no return value, so even
//!   a cloned registry yields nothing recoverable.
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

    /// Register one per-account sender into its host wrapper. This is
    /// the ONLY typed mutation surface, and it is exfiltration-free by
    /// construction: no caller-supplied closure ever runs against the
    /// concrete wrapper, the account value is consumed, and nothing is
    /// returned — so cloning the registry from a `&self` context gains
    /// an attacker nothing (round-3 review: the former Clone +
    /// closure-based generic accessor allowed recovering a concrete
    /// sender from dispatch paths, so the generic accessor was
    /// deleted). Panics only on a construction bug (catalog did not
    /// register the host wrapper).
    pub(crate) fn register<A: AccountRegistration>(&mut self, account: A) {
        let host = self
            .senders
            .iter_mut()
            .find_map(|sender| sender.as_any_mut().downcast_mut::<A::Host>())
            .expect("builtin sender registered at construction");
        account.register_into(host);
    }
}

/// Implemented by each channel's per-account sender in the channel's
/// own module: names the registry wrapper it registers into and
/// performs the registration. The trait deliberately consumes the
/// account and returns nothing, and it is SEALED: only the account
/// types listed in [`private`] below may implement it, so dispatcher
/// code cannot smuggle a concrete wrapper out through an ad-hoc
/// implementation with side-channel state.
pub(crate) trait AccountRegistration: private::Sealed {
    type Host: OutboundChannelSender + Clone + 'static;
    fn register_into(self, host: &mut Self::Host);
}

mod private {
    pub trait Sealed {}

    impl Sealed for crate::telegram::outbound::TelegramSender {}
    impl Sealed for crate::discord::outbound::DiscordSender {}
    impl Sealed for crate::feishu::outbound::FeishuSender {}
    impl Sealed for crate::weixin::outbound::WeixinSender {}
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
