//! Weixin outbound send paths: text/media/typing/subscription API
//! calls and their body builders. Moved verbatim from weixin.rs
//! (Phase-7 pure code motion).

use super::*;

pub(super) fn build_send_text_message_body(
    to_user_id: &str,
    text: &str,
    context_token: &str,
    client_id: &str,
    message_state: u8,
) -> Value {
    json!({
        "msg": {
            "from_user_id": "",
            "to_user_id": to_user_id,
            "client_id": client_id,
            "message_type": 2,
            "message_state": message_state,
            "context_token": context_token,
            "item_list": [
                {
                    "type": 1,
                    "text_item": {
                        "text": text
                    }
                }
            ]
        },
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub(super) async fn send_text_message_with_state(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
    client_id: &str,
    message_state: u8,
) -> Result<(), ChannelError> {
    // SDK parity: refuse to send if session is paused (errcode=-14).
    let acct_id = account_id_from_token(&account.token);
    if !acct_id.is_empty() && is_session_paused(acct_id).await {
        return Err(ChannelError::SendFailed(
            "Weixin session paused (errcode=-14); suppressing send".to_owned(),
        ));
    }

    let target = to_user_id.trim();
    if target.is_empty() {
        return Err(ChannelError::Config(
            "Weixin target user_id is empty".to_owned(),
        ));
    }

    let token = context_token.map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin context_token is missing; cannot send reply".to_owned(),
        ));
    }

    // Check token send counter — refuse if already at limit
    let remaining = token_sends_remaining(token).await;
    if remaining == 0 {
        warn!(
            to_user_id = target,
            token_limit = TOKEN_SEND_LIMIT,
            "context_token exhausted (hit send limit), refusing send"
        );
        return Err(ChannelError::SendFailed(
            "Weixin context_token exhausted (send limit reached)".to_owned(),
        ));
    }

    let body = build_send_text_message_body(target, text, token, client_id, message_state);

    let url = build_api_url(&account.base_url, "sendmessage");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendmessage failed: {error}")))?;

    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendmessage HTTP {status}: {raw}"
        )));
    }

    if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
        let ret = payload.get("ret").and_then(Value::as_i64);
        if ret.is_some_and(|r| r != 0) {
            let ret_code = ret.unwrap_or(-1);
            let message = payload
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            // ret=-14: session expired — bot needs re-login on the phone.
            // SDK parity: pause all API calls for this account for 1 hour.
            if ret_code == -14 {
                error!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin session expired (ret=-14): bot needs re-login on the phone"
                );
                if !acct_id.is_empty() {
                    pause_session(acct_id).await;
                }
            }
            // ret=-2: parameter error, typically context_token expired or exhausted
            if ret_code == -2 {
                warn!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin context_token likely expired or exhausted (ret=-2)"
                );
            }

            tracing::warn!(
                ret = ret_code,
                errmsg = message,
                to_user_id = target,
                text_len = text.len(),
                response_body = %raw,
                "Weixin sendmessage (text) failed"
            );
            return Err(ChannelError::SendFailed(format!(
                "Weixin sendmessage ret={ret_code}: {message}"
            )));
        }
    }

    // Successful send — increment the token's usage counter
    let count = token_send_increment(token).await;
    if let Some(c) = count {
        let left = TOKEN_SEND_LIMIT.saturating_sub(c);
        if left <= 2 {
            warn!(
                to_user_id = target,
                sends_used = c,
                sends_remaining = left,
                "context_token nearing send limit"
            );
        }
    }

    Ok(())
}

pub async fn send_text_message(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    let client_id = uuid::Uuid::new_v4().to_string();
    send_text_message_with_state(
        http,
        account,
        to_user_id,
        text,
        context_token,
        &client_id,
        2,
    )
    .await?;
    Ok(client_id)
}

pub(super) async fn fetch_typing_ticket(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    context_token: Option<&str>,
) -> Result<Option<String>, ChannelError> {
    let body = build_get_config_body(to_user_id, context_token);
    let url = build_api_url(&account.base_url, "getconfig");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin getconfig failed: {error}")))?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getconfig HTTP {status}: {raw}"
        )));
    }
    let payload: WeixinGetConfigResp = serde_json::from_str(&raw).map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin getconfig parse failed: {error}; body={raw}"
        ))
    })?;
    if payload.ret != 0 {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getconfig ret!=0: {}",
            payload.errmsg
        )));
    }
    let ticket = payload.typing_ticket.trim().to_owned();
    if ticket.is_empty() {
        return Ok(None);
    }
    Ok(Some(ticket))
}

pub(super) async fn send_typing_status(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    typing_ticket: &str,
    status: i64,
) -> Result<(), ChannelError> {
    if typing_ticket.trim().is_empty() {
        return Ok(());
    }
    let body = build_send_typing_body(to_user_id, typing_ticket, status);
    let url = build_api_url(&account.base_url, "sendtyping");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendtyping failed: {error}")))?;
    let status_code = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status_code.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendtyping HTTP {status_code}: {raw}"
        )));
    }
    if let Ok(payload) = serde_json::from_str::<Value>(&raw)
        && payload
            .get("ret")
            .and_then(Value::as_i64)
            .is_some_and(|ret| ret != 0)
    {
        let message = payload
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendtyping ret!=0: {message}"
        )));
    }
    Ok(())
}

pub(super) async fn notify_subscription(
    http: &Client,
    account: &WeixinAccount,
    endpoint: &str,
) -> Result<(), ChannelError> {
    let body = json!({
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    });
    let url = build_api_url(&account.base_url, endpoint);
    let response = auth_headers(
        http.post(url).timeout(Duration::from_secs(2)).json(&body),
        account,
    )
    .send()
    .await
    .map_err(|error| ChannelError::SendFailed(format!("Weixin {endpoint} failed: {error}")))?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin {endpoint} HTTP {status}: {raw}"
        )));
    }
    if let Ok(payload) = serde_json::from_str::<Value>(&raw)
        && payload
            .get("ret")
            .and_then(Value::as_i64)
            .is_some_and(|ret| ret != 0)
    {
        let message = payload
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(ChannelError::SendFailed(format!(
            "Weixin {endpoint} ret!=0: {message}"
        )));
    }
    Ok(())
}

pub(super) async fn notify_start(
    http: &Client,
    account: &WeixinAccount,
) -> Result<(), ChannelError> {
    notify_subscription(http, account, "msg/notifystart").await
}

pub(super) async fn notify_stop(
    http: &Client,
    account: &WeixinAccount,
) -> Result<(), ChannelError> {
    notify_subscription(http, account, "msg/notifystop").await
}

pub(super) fn build_get_config_body(to_user_id: &str, context_token: Option<&str>) -> Value {
    let mut body = json!({
        "ilink_user_id": to_user_id,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    });
    if let Some(token) = context_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["context_token"] = Value::String(token.to_owned());
    }
    body
}

pub(super) fn build_send_typing_body(to_user_id: &str, typing_ticket: &str, status: i64) -> Value {
    json!({
        "ilink_user_id": to_user_id,
        "typing_ticket": typing_ticket,
        "status": status,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn get_upload_url(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    media_type: i64,
    filekey: &str,
    plaintext: &[u8],
    ciphertext_size: usize,
    aes_key_hex: &str,
) -> Result<UploadUrlResult, ChannelError> {
    let body = build_get_upload_url_body(
        filekey,
        to_user_id,
        media_type,
        plaintext,
        ciphertext_size,
        aes_key_hex,
    );
    let url = build_api_url(&account.base_url, "getuploadurl");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| {
            ChannelError::SendFailed(format!("Weixin getuploadurl failed: {error}"))
        })?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getuploadurl HTTP {status}: {raw}"
        )));
    }
    let payload: WeixinGetUploadUrlResp = serde_json::from_str(&raw).map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin getuploadurl parse failed: {error}; body={raw}"
        ))
    })?;
    if payload.ret != 0 {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getuploadurl ret!=0: {}",
            payload.errmsg
        )));
    }
    // SDK parity: prefer upload_full_url when present; fall back to upload_param.
    if let Some(full_url) = payload
        .upload_full_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        return Ok(UploadUrlResult::FullUrl(full_url.to_owned()));
    }
    let upload_param = payload.upload_param.trim().to_owned();
    if upload_param.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin getuploadurl returned empty upload_param and no upload_full_url".to_owned(),
        ));
    }
    Ok(UploadUrlResult::Param(upload_param))
}

pub(super) fn build_get_upload_url_body(
    filekey: &str,
    to_user_id: &str,
    media_type: i64,
    plaintext: &[u8],
    ciphertext_size: usize,
    aes_key_hex: &str,
) -> Value {
    json!({
        "filekey": filekey,
        "media_type": media_type,
        "to_user_id": to_user_id,
        "rawsize": plaintext.len(),
        "rawfilemd5": format!("{:x}", md5::compute(plaintext)),
        "filesize": ciphertext_size,
        "no_need_thumb": true,
        "aeskey": aes_key_hex,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub(super) fn build_send_media_message_body(
    to_user_id: &str,
    context_token: &str,
    client_id: &str,
    uploaded: &UploadedWeixinMedia,
) -> Value {
    // Reference SDK parity: outbound `media.aes_key` is base64(hex(aes_key_raw)),
    // not base64(raw bytes).
    let aes_key_outbound = STANDARD.encode(bytes_to_hex(&uploaded.aes_key_raw).as_bytes());
    let media = json!({
        "encrypt_query_param": uploaded.download_encrypted_query_param,
        "aes_key": aes_key_outbound,
        "encrypt_type": 1
    });
    let media_item = match uploaded.media_type {
        2 => json!({
            "type": 5,
            "video_item": {
                "media": media,
                "video_size": uploaded.ciphertext_size
            }
        }),
        3 => json!({
            "type": 4,
            "file_item": {
                "media": media,
                "file_name": uploaded
                    .file_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("attachment.bin"),
                "len": uploaded.plaintext_size.to_string()
            }
        }),
        _ => json!({
            "type": 2,
            "image_item": {
                "media": media,
                "mid_size": uploaded.ciphertext_size
            }
        }),
    };
    json!({
        "msg": {
            "from_user_id": "",
            "to_user_id": to_user_id,
            "client_id": client_id,
            "message_type": 2,
            "message_state": 2,
            "context_token": context_token,
            "item_list": [media_item]
        },
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub(super) async fn send_media_message(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    uploaded: &UploadedWeixinMedia,
    text: &str,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    // SDK parity: refuse to send if session is paused (errcode=-14).
    let acct_id = account_id_from_token(&account.token);
    if !acct_id.is_empty() && is_session_paused(acct_id).await {
        return Err(ChannelError::SendFailed(
            "Weixin session paused (errcode=-14); suppressing media send".to_owned(),
        ));
    }

    let target = to_user_id.trim();
    if target.is_empty() {
        return Err(ChannelError::Config(
            "Weixin target user_id is empty".to_owned(),
        ));
    }
    let token = context_token.map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin context_token is missing; cannot send reply".to_owned(),
        ));
    }

    // Check token send counter — refuse if exhausted
    let remaining = token_sends_remaining(token).await;
    if remaining == 0 {
        warn!(
            to_user_id = target,
            token_limit = TOKEN_SEND_LIMIT,
            "context_token exhausted (hit send limit), refusing media send"
        );
        return Err(ChannelError::SendFailed(
            "Weixin context_token exhausted (send limit reached)".to_owned(),
        ));
    }

    // Reference SDK parity: send caption text as a standalone text message first,
    // then send media with a single item in item_list.
    if !text.trim().is_empty() {
        send_text_message(http, account, target, text, Some(token)).await?;
    }
    let client_id = uuid::Uuid::new_v4().to_string();
    let body = build_send_media_message_body(target, token, &client_id, uploaded);

    let url = build_api_url(&account.base_url, "sendmessage");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendmessage failed: {error}")))?;

    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendmessage HTTP {status}: {raw}"
        )));
    }

    if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
        let ret = payload.get("ret").and_then(Value::as_i64);
        if ret.is_some_and(|r| r != 0) {
            let ret_code = ret.unwrap_or(-1);
            let message = payload
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if ret_code == -14 {
                error!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin session expired during media send (ret=-14): bot needs re-login on the phone"
                );
                if !acct_id.is_empty() {
                    pause_session(acct_id).await;
                }
            }
            if ret_code == -2 {
                warn!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin context_token likely expired or exhausted during media send (ret=-2)"
                );
            }
            tracing::warn!(
                ret = ret_code,
                errmsg = message,
                response_body = %raw,
                "Weixin sendmessage (media) failed"
            );
            return Err(ChannelError::SendFailed(format!(
                "Weixin sendmessage ret={ret_code}: {message}"
            )));
        }
    }

    // Successful media send — increment token counter
    let count = token_send_increment(token).await;
    if let Some(c) = count {
        let left = TOKEN_SEND_LIMIT.saturating_sub(c);
        if left <= 2 {
            warn!(
                to_user_id = target,
                sends_used = c,
                sends_remaining = left,
                "context_token nearing send limit (media)"
            );
        }
    }

    Ok(client_id)
}

pub async fn send_image_message_from_path(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    image_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    send_image_message_from_path_with_cdn_base(
        http,
        account,
        to_user_id,
        image_path,
        caption,
        context_token,
        DEFAULT_WEIXIN_CDN_BASE_URL,
    )
    .await
}

pub async fn send_image_message_from_path_with_cdn_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    image_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
    cdn_base_url: &str,
) -> Result<String, ChannelError> {
    if !image_path.is_absolute() {
        return Err(ChannelError::SendFailed(
            "Weixin image path must be absolute".to_owned(),
        ));
    }
    let image_bytes = fs::read(image_path).await.map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin image read failed ({}): {error}",
            image_path.display()
        ))
    })?;
    let media_type = classify_media_type_from_url(image_path.to_string_lossy().as_ref());
    let uploaded = upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        &image_bytes,
        media_type,
        cdn_base_url,
        // Images render by their encrypted thumbnail, not filename — leave None.
        None,
    )
    .await?;
    send_media_message(
        http,
        account,
        to_user_id,
        &uploaded,
        caption.unwrap_or_default(),
        context_token,
    )
    .await
}

pub async fn send_file_message_from_path(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    file_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    send_file_message_from_path_with_cdn_base(
        http,
        account,
        to_user_id,
        file_path,
        caption,
        context_token,
        DEFAULT_WEIXIN_CDN_BASE_URL,
    )
    .await
}

pub async fn send_file_message_from_path_with_cdn_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    file_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
    cdn_base_url: &str,
) -> Result<String, ChannelError> {
    if !file_path.is_absolute() {
        return Err(ChannelError::SendFailed(
            "Weixin file path must be absolute".to_owned(),
        ));
    }
    let file_bytes = fs::read(file_path).await.map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin file read failed ({}): {error}",
            file_path.display()
        ))
    })?;
    let media_type = classify_media_type_from_url(file_path.to_string_lossy().as_ref());
    // Preserve the original file name so WeChat renders "foo.pdf" instead of
    // "attachment.bin" in the chat bubble. Fall back to None for paths whose
    // OsStr isn't valid UTF-8 — callers can still rely on the "attachment.bin"
    // default in `build_send_media_message_body`.
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_owned());
    let uploaded = upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        &file_bytes,
        media_type,
        cdn_base_url,
        file_name,
    )
    .await?;
    send_media_message(
        http,
        account,
        to_user_id,
        &uploaded,
        caption.unwrap_or_default(),
        context_token,
    )
    .await
}
