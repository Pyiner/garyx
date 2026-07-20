//! Compile-time probe for the public Discord channel surface: the domain
//! split must keep every pre-split public path importable from outside the
//! crate. This file intentionally only imports; a private symbol becomes an
//! E0603 compile error.

#[allow(unused_imports)]
use garyx_channels::{
    DiscordChannel as RootDiscordChannel, DiscordSender as RootDiscordSender,
    discord::{
        DiscordChannel as ModuleDiscordChannel,
        outbound::{DiscordChannelSender, DiscordSender as ModuleDiscordSender},
    },
    dispatcher::DiscordSender as DispatcherDiscordSender,
};

#[test]
fn discord_public_surface_is_importable() {}
