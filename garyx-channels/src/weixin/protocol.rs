//! Weixin wire protocol: API/CDN URL builders, auth headers, AES
//! crypto helpers, and the get-updates/upload wire structs. Moved
//! verbatim from weixin.rs (Phase-7 pure code motion).

use super::*;

pub(super) type Aes128EcbEnc = Encryptor<Aes128>;
pub(super) type Aes128EcbDec = Decryptor<Aes128>;

/// Extract account_id from a weixin bot token.
/// Token format is typically `"account_id@im.bot:secret"`.
pub(super) fn account_id_from_token(token: &str) -> &str {
    token.split(':').next().unwrap_or(token).trim()
}

pub(super) fn build_api_url(base_url: &str, endpoint: &str) -> String {
    format!(
        "{}/ilink/bot/{endpoint}",
        base_url.trim_end_matches('/').trim_end()
    )
}

pub(super) fn build_cdn_upload_url(
    cdn_base_url: &str,
    upload_param: &str,
    filekey: &str,
) -> String {
    format!(
        "{}/upload?encrypted_query_param={}&filekey={}",
        cdn_base_url.trim_end_matches('/').trim_end(),
        urlencoding::encode(upload_param),
        urlencoding::encode(filekey)
    )
}

pub(super) fn build_cdn_download_url(cdn_base_url: &str, encrypted_query_param: &str) -> String {
    format!(
        "{}/download?encrypted_query_param={}",
        cdn_base_url.trim_end_matches('/').trim_end(),
        urlencoding::encode(encrypted_query_param)
    )
}

pub(super) fn auth_headers(builder: RequestBuilder, account: &WeixinAccount) -> RequestBuilder {
    let wechat_uin = if account.uin.trim().is_empty() {
        random_wechat_uin()
    } else {
        account.uin.trim().to_owned()
    };

    builder
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {}", account.token.trim()))
        .header("X-WECHAT-UIN", wechat_uin)
        // Reference SDK parity: always send app-id and client version headers.
        // The SDK packs the version as (major << 16) | (minor << 8) | patch.
        .header("iLink-App-Id", "bot")
        .header("iLink-App-ClientVersion", build_client_version())
}

/// Build a packed uint32 client version matching the SDK's format:
/// `(major << 16) | (minor << 8) | patch`.
pub(super) fn build_client_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let parts: Vec<u32> = version
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);
    ((major << 16) | (minor << 8) | patch).to_string()
}

pub(super) fn random_wechat_uin() -> String {
    let uuid_bytes = uuid::Uuid::new_v4();
    let bytes = uuid_bytes.as_bytes();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    STANDARD.encode(value.to_string())
}

pub(super) fn random_16_bytes() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

pub(super) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(super) fn parse_hex_key_16(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.trim();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    let bytes = hex.as_bytes();
    for index in 0..16 {
        let hi = bytes[index * 2] as char;
        let lo = bytes[index * 2 + 1] as char;
        let pair = [hi, lo].iter().collect::<String>();
        out[index] = u8::from_str_radix(&pair, 16).ok()?;
    }
    Some(out)
}

pub(super) fn aes_ecb_padded_size(plaintext_size: usize) -> usize {
    ((plaintext_size + 1).div_ceil(16)) * 16
}

pub(super) fn encrypt_aes_ecb(plaintext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ChannelError> {
    let enc = Aes128EcbEnc::new_from_slice(key)
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes init failed: {error}")))?;
    let block_size = 16_usize;
    let padded_len = aes_ecb_padded_size(plaintext.len());
    let mut buffer = vec![0_u8; padded_len.max(block_size)];
    buffer[..plaintext.len()].copy_from_slice(plaintext);
    let encrypted = enc
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes encrypt failed: {error}")))?;
    Ok(encrypted.to_vec())
}

pub(super) fn decrypt_aes_ecb(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ChannelError> {
    let dec = Aes128EcbDec::new_from_slice(key)
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes init failed: {error}")))?;
    let mut buffer = ciphertext.to_vec();
    dec.decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map(|value| value.to_vec())
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes decrypt failed: {error}")))
}

pub(super) fn parse_aes_key_base64(aes_key_base64: &str) -> Option<[u8; 16]> {
    let decoded = STANDARD.decode(aes_key_base64).ok()?;
    if decoded.len() == 16 {
        return decoded.try_into().ok();
    }
    if decoded.len() == 32 {
        let as_text = std::str::from_utf8(&decoded).ok()?;
        return parse_hex_key_16(as_text);
    }
    None
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinTextItem {
    #[serde(default)]
    pub(super) text: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinVoiceItem {
    #[serde(default)]
    pub(super) text: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub(super) media: Option<WeixinCdnMedia>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinImageItem {
    #[serde(default)]
    pub(super) url: String,
    #[serde(default)]
    pub(super) media: Option<WeixinCdnMedia>,
    #[serde(default)]
    pub(super) aeskey: String,
    #[serde(default)]
    #[allow(dead_code)] // serde-populated, used in forwarded JSON
    pub(super) mid_size: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinFileItem {
    #[serde(default)]
    pub(super) media: Option<WeixinCdnMedia>,
    #[serde(default)]
    pub(super) file_name: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinVideoItem {
    #[serde(default)]
    pub(super) media: Option<WeixinCdnMedia>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinCdnMedia {
    #[serde(default)]
    pub(super) encrypt_query_param: String,
    #[serde(default)]
    pub(super) aes_key: String,
    #[serde(default)]
    #[allow(dead_code)] // serde-populated, used in forwarded JSON
    pub(super) encrypt_type: i64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinRefMessage {
    #[serde(default)]
    pub(super) title: String,
    #[serde(default)]
    pub(super) message_item: Option<Box<WeixinMessageItem>>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinMessageItem {
    #[serde(default)]
    pub(super) r#type: i64,
    #[serde(default)]
    pub(super) ref_msg: Option<WeixinRefMessage>,
    #[serde(default)]
    pub(super) text_item: Option<WeixinTextItem>,
    #[serde(default)]
    pub(super) image_item: Option<WeixinImageItem>,
    #[serde(default)]
    pub(super) voice_item: Option<WeixinVoiceItem>,
    #[serde(default)]
    pub(super) file_item: Option<WeixinFileItem>,
    #[serde(default)]
    pub(super) video_item: Option<WeixinVideoItem>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct WeixinMessage {
    #[serde(default)]
    pub(super) message_type: i64,
    #[serde(default)]
    pub(super) from_user_id: String,
    #[serde(default)]
    pub(super) context_token: String,
    #[serde(default)]
    pub(super) item_list: Vec<WeixinMessageItem>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WeixinGetUpdatesResp {
    #[serde(default)]
    pub(super) ret: i64,
    #[serde(default)]
    pub(super) errcode: i64,
    #[serde(default)]
    pub(super) errmsg: String,
    #[serde(default)]
    pub(super) msgs: Vec<WeixinMessage>,
    #[serde(default)]
    pub(super) get_updates_buf: String,
    #[serde(default)]
    pub(super) longpolling_timeout_ms: u64,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WeixinGetUploadUrlResp {
    #[serde(default)]
    pub(super) ret: i64,
    #[serde(default)]
    pub(super) errmsg: String,
    #[serde(default)]
    pub(super) upload_param: String,
    /// SDK parity: server may return a complete upload URL instead of just upload_param.
    #[serde(default)]
    pub(super) upload_full_url: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WeixinGetConfigResp {
    #[serde(default)]
    pub(super) ret: i64,
    #[serde(default)]
    pub(super) errmsg: String,
    #[serde(default)]
    pub(super) typing_ticket: String,
}

#[derive(Debug, Clone)]
pub(super) struct UploadedWeixinMedia {
    pub(super) download_encrypted_query_param: String,
    pub(super) aes_key_raw: [u8; 16],
    pub(super) plaintext_size: usize,
    pub(super) ciphertext_size: usize,
    pub(super) media_type: i64,
    /// Original file name for type=3 file messages. When `None`, falls back to
    /// `"attachment.bin"` in `build_send_media_message_body`. WeChat uses this
    /// field to render the filename + icon in the chat bubble; without it the
    /// recipient sees a useless "attachment.bin".
    pub(super) file_name: Option<String>,
}

#[derive(Debug)]
pub(super) enum UploadUrlResult {
    /// Server returned a complete upload URL (preferred).
    FullUrl(String),
    /// Server returned upload_param for client-side URL construction.
    Param(String),
}
