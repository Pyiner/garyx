//! Compile-time probe for the `garyx_channels::weixin` public surface
//! (Phase-7): the domain split must keep every pre-split public path
//! importable from outside the crate. This file intentionally only
//! imports — a private symbol turns it into an E0603 compile error.

#[allow(unused_imports)]
use garyx_channels::weixin::{
    PendingOutboundMessage, WeixinChannel, clear_session_pause, drain_pending_outbound,
    get_context_token, get_context_token_for_thread, get_typing_ticket, is_session_paused,
    pause_session, pending_outbound_count, queue_pending_outbound, send_file_message_from_path,
    send_file_message_from_path_with_cdn_base, send_image_message_from_path,
    send_image_message_from_path_with_cdn_base, send_text_message, set_context_token,
    set_context_token_for_thread, set_typing_ticket, token_send_count_prune,
    token_send_count_reset, token_send_increment, token_sends_remaining,
};

#[test]
fn weixin_public_surface_is_importable() {}
