//! Channel-plugin host runtime.
//!
//! Spawns each plugin as a child process and talks JSON-RPC 2.0 over stdio framed
//! with LSP-style `Content-Length:` headers.
//!
//! This module is the *host* side of that protocol. It is intentionally
//! agnostic to:
//! - the transport (any `AsyncRead + AsyncWrite` works; we use child
//!   stdio in production and `tokio::io::duplex` in tests),
//! - the upper layer (the `SubprocessPlugin` that glues this to
//!   `ChannelPluginManager` lives above, not here).
//!
//! Module tree:
//! - [`codec`]: LSP framing + JSON-RPC envelope.
//! - [`protocol`]: wire types for every RPC.
//! - [`manifest`]: `plugin.toml` parsing.
//! - [`stream`]: server-assigned `stream_id`s + tombstone registry.
//! - [`transport`]: reader/writer pumps, notification routing, and the
//!   [`transport::PluginRpcClient`] handle used by the rest of the host.

pub mod auth_flow_bridge;
pub mod codec;
pub mod discoverer;
pub mod inspect;
pub mod manifest;
pub mod preflight;
pub mod protocol;
pub mod sender;
pub mod stream;
pub mod subprocess;
pub mod subprocess_plugin;
pub mod transport;

pub use auth_flow_bridge::SubprocessAuthFlowExecutor;
pub use codec::{CodecError, FrameCodec, MAX_FRAME_BYTES_DEFAULT};
pub use discoverer::{DiscoveryError, DiscoveryOutcome, ManifestDiscoverer};
pub use inspect::{InspectError, InspectReport, inspect, synthesize_manifest_toml};
pub use manifest::{
    AccountRootBehavior, AuthFlowDescriptor, DeliveryModel, ManifestCapabilities, ManifestError,
    ManifestRuntime, PluginManifest, PluginUi,
};
pub use preflight::{PROTOCOL_VERSION, PreflightFailure, PreflightSummary, preflight};
pub use protocol::{
    AccountDescriptor, AttachmentRef, AuthFlowDisplayItem, AuthFlowPollResponse,
    AuthFlowStartRequest, AuthFlowStartResponse, CapabilitiesResponse, DispatchOutbound,
    DispatchOutboundResult, HostContext, InboundEnd, InboundEndStatus, InboundRequestPayload,
    InitializeParams, InitializeResult, PluginErrorCode, RecordOutbound, ReloadAccountsParams,
    ResolveAccountUiParams, ResolveAccountUiResult, StreamEventFrame, StreamFrameParams,
    UiConversationNode, UiEndpointDescriptor,
};
pub use sender::{DISPATCH_TIMEOUT, PluginSenderHandle};
pub use stream::{StreamId, StreamIdGenerator, StreamRegistry, TombstoneReason};
pub use subprocess::{ExitReport, SpawnOptions, SubprocessError, SubprocessPlugin};
pub use subprocess_plugin::SubprocessChannelPlugin;
pub use transport::{
    InboundHandler, PluginRpcClient, RpcError, Transport, TransportConfig, TransportHandles,
};
