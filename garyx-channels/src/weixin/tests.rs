use super::*;
use crate::streaming_core::BoundaryTextEffect;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn weixin_test_account(api: &MockServer, token: &str) -> WeixinAccount {
    WeixinAccount {
        token: token.to_owned(),
        uin: String::new(),
        enabled: true,
        base_url: api.uri(),
        name: None,
        agent_id: "claude".to_owned(),
        workspace_dir: None,
        streaming_update: true,
    }
}

#[test]
fn test_extract_text_combines_text_and_voice_transcript() {
    let items = vec![
        WeixinMessageItem {
            r#type: 1,
            ref_msg: None,
            text_item: Some(WeixinTextItem {
                text: "hello".to_owned(),
            }),
            image_item: None,
            voice_item: None,
            ..Default::default()
        },
        WeixinMessageItem {
            r#type: 3,
            ref_msg: None,
            text_item: None,
            image_item: None,
            voice_item: Some(WeixinVoiceItem {
                text: "world".to_owned(),
                media: None,
            }),
            ..Default::default()
        },
    ];
    assert_eq!(extract_text(&items), "hello\nworld");
}

#[test]
fn test_extract_text_includes_quote_context() {
    let items = vec![WeixinMessageItem {
        r#type: 1,
        ref_msg: Some(WeixinRefMessage {
            title: "上一条".to_owned(),
            message_item: Some(Box::new(WeixinMessageItem {
                r#type: 1,
                ref_msg: None,
                text_item: Some(WeixinTextItem {
                    text: "引用内容".to_owned(),
                }),
                image_item: None,
                voice_item: None,
                ..Default::default()
            })),
        }),
        text_item: Some(WeixinTextItem {
            text: "回复内容".to_owned(),
        }),
        image_item: None,
        voice_item: None,
        ..Default::default()
    }];

    assert_eq!(extract_text(&items), "[引用: 上一条 | 引用内容]\n回复内容");
}

#[test]
fn test_extract_text_keeps_image_message_as_placeholder() {
    let items = vec![WeixinMessageItem {
        r#type: 2,
        ref_msg: None,
        text_item: None,
        image_item: Some(WeixinImageItem {
            url: "https://example.com/a.png".to_owned(),
            ..Default::default()
        }),
        voice_item: None,
        ..Default::default()
    }];
    assert_eq!(extract_text(&items), "[图片] https://example.com/a.png");
}

#[test]
fn test_build_api_url() {
    assert_eq!(
        build_api_url("https://ilinkai.weixin.qq.com", "sendmessage"),
        "https://ilinkai.weixin.qq.com/ilink/bot/sendmessage"
    );
    assert_eq!(
        build_api_url("https://ilinkai.weixin.qq.com/", "getupdates"),
        "https://ilinkai.weixin.qq.com/ilink/bot/getupdates"
    );
}

#[test]
fn test_outbound_media_ref_file_name_extraction() {
    // Plain local path — straightforward basename extraction.
    let basename = "resume.pdf";
    assert_eq!(
        OutboundMediaRef::LocalPath(format!("/tmp/{basename}")).file_name(),
        Some(basename.to_owned())
    );
    // HTTPS URL with a query string — strip `?...` before basename.
    assert_eq!(
        OutboundMediaRef::RemoteUrl("https://cdn.example.com/a/b/report.pdf?sig=xyz".to_owned())
            .file_name(),
        Some("report.pdf".to_owned())
    );
    // Trailing slash — `rsplit` should still find the previous segment.
    assert_eq!(
        OutboundMediaRef::RemoteUrl("https://example.com/folder/".to_owned()).file_name(),
        Some("folder".to_owned())
    );
    // No path component — nothing usable to send.
    assert_eq!(
        OutboundMediaRef::RemoteUrl("https://example.com".to_owned()).file_name(),
        Some("example.com".to_owned())
    );
}

#[test]
fn test_extract_markdown_media_refs_supports_remote_and_local() {
    let refs =
        extract_markdown_media_refs("hi ![a](https://example.com/a.png) ![b](file:///tmp/a.png)");
    assert_eq!(refs.len(), 2);
    assert!(
        refs.iter()
            .any(|item| matches!(item, OutboundMediaRef::RemoteUrl(url) if url == "https://example.com/a.png"))
    );
    assert!(
        refs.iter()
            .any(|item| matches!(item, OutboundMediaRef::LocalPath(path) if path == "/tmp/a.png"))
    );
}

#[test]
fn test_extract_media_refs_from_value_supports_local_and_remote() {
    let payload = json!({
        "result": {
            "image_path": "/tmp/output.png",
            "image_url": "https://example.com/final.jpg",
        }
    });
    let mut refs = Vec::new();
    extract_media_refs_from_value(&payload, &mut refs, 8);
    assert!(refs.len() >= 2);
    assert!(refs.iter().any(
        |item| matches!(item, OutboundMediaRef::LocalPath(path) if path == "/tmp/output.png")
    ));
    assert!(
        refs.iter()
            .any(|item| matches!(item, OutboundMediaRef::RemoteUrl(url) if url == "https://example.com/final.jpg"))
    );
}

#[test]
fn test_extract_media_refs_from_provider_message_supports_image_generation_base64() {
    let message = garyx_models::provider::ProviderMessage::tool_result(
        json!({
            "type": "imageGeneration",
            "id": "ig-weixin-test",
            "status": "completed",
            "result": "iVBORw0KGgo=",
        }),
        Some("ig-weixin-test".to_owned()),
        Some("imageGeneration".to_owned()),
        Some(false),
    )
    .with_metadata_value("item_type", json!("imageGeneration"));

    let refs = extract_media_refs_from_provider_message(&message);
    assert_eq!(refs.len(), 1);
    match &refs[0] {
        OutboundMediaRef::InlineImage {
            id,
            bytes,
            file_name,
        } => {
            assert_eq!(id, "ig-weixin-test");
            assert_eq!(bytes, &vec![137, 80, 78, 71, 13, 10, 26, 10]);
            assert_eq!(file_name, "ig-weixin-test.png");
        }
        other => panic!("expected inline image ref, got {other:?}"),
    }
}

#[tokio::test]
async fn test_load_media_bytes_supports_inline_image_generation() {
    let media_ref = OutboundMediaRef::InlineImage {
        id: "ig-inline".to_owned(),
        bytes: vec![1, 2, 3, 4],
        file_name: "ig-inline.png".to_owned(),
    };
    let bytes = load_media_bytes(&reqwest::Client::new(), &media_ref)
        .await
        .unwrap();
    assert_eq!(bytes, vec![1, 2, 3, 4]);
    assert_eq!(media_ref.classify_media_type(), 1);
    assert_eq!(media_ref.file_name().as_deref(), Some("ig-inline.png"));
}

#[test]
fn test_markdown_to_plain_text_strips_image_and_link_markup() {
    let plain = markdown_to_plain_text("![img](https://a)\n[txt](https://b)");
    assert_eq!(plain.trim(), "txt");
}

#[test]
fn test_weixin_stream_boundary_user_ack_clears_buffer() {
    let mut stream_text = "buffered text".to_owned();
    let effect = apply_weixin_stream_boundary(&mut stream_text, StreamBoundaryKind::UserAck);
    assert_eq!(effect, BoundaryTextEffect::Cleared);
    assert!(stream_text.is_empty());
}

#[test]
fn test_weixin_stream_boundary_assistant_segment_appends_separator() {
    let mut stream_text = "assistant chunk".to_owned();
    let effect =
        apply_weixin_stream_boundary(&mut stream_text, StreamBoundaryKind::AssistantSegment);
    assert_eq!(effect, BoundaryTextEffect::AssistantSeparatorAppended);
    assert_eq!(stream_text, "assistant chunk\n\n");
}

#[tokio::test]
async fn test_context_token_store_prefers_thread_token() {
    let account_id = format!("acc-{}", uuid::Uuid::new_v4());
    let user_id = format!("user-{}", uuid::Uuid::new_v4());
    let thread_a = format!("thread::{}", uuid::Uuid::new_v4());
    let thread_b = format!("thread::{}", uuid::Uuid::new_v4());
    set_context_token_for_thread(&account_id, &user_id, None, "user-token").await;
    set_context_token_for_thread(&account_id, &user_id, Some(&thread_a), "thread-a-token").await;
    set_context_token_for_thread(&account_id, &user_id, Some(&thread_b), "thread-b-token").await;

    let token_a = get_context_token_for_thread(&account_id, &user_id, Some(&thread_a)).await;
    let token_b = get_context_token_for_thread(&account_id, &user_id, Some(&thread_b)).await;
    let token_user = get_context_token_for_thread(&account_id, &user_id, None).await;
    assert_eq!(token_a.as_deref(), Some("thread-a-token"));
    assert_eq!(token_b.as_deref(), Some("thread-b-token"));
    assert_eq!(token_user.as_deref(), Some("user-token"));
}

#[tokio::test]
async fn test_context_token_store_falls_back_to_user_token() {
    let account_id = format!("acc-{}", uuid::Uuid::new_v4());
    let user_id = format!("user-{}", uuid::Uuid::new_v4());
    let thread_id = format!("thread::{}", uuid::Uuid::new_v4());
    set_context_token_for_thread(&account_id, &user_id, None, "user-only-token").await;
    let token = get_context_token_for_thread(&account_id, &user_id, Some(&thread_id)).await;
    assert_eq!(token.as_deref(), Some("user-only-token"));
}

#[test]
fn test_diff_ref_sdk_random_wechat_uin_format() {
    // reference SDK parity: X-WECHAT-UIN is base64(decimal_u32_string)
    let encoded = random_wechat_uin();
    let decoded = STANDARD.decode(encoded).expect("base64 decode");
    let text = String::from_utf8(decoded).expect("utf8");
    assert!(
        text.parse::<u32>().is_ok(),
        "decoded value should be u32 decimal"
    );
}

#[test]
fn test_diff_ref_sdk_aes_ecb_padded_size() {
    assert_eq!(aes_ecb_padded_size(0), 16);
    assert_eq!(aes_ecb_padded_size(1), 16);
    assert_eq!(aes_ecb_padded_size(15), 16);
    assert_eq!(aes_ecb_padded_size(16), 32);
    assert_eq!(aes_ecb_padded_size(17), 32);
}

#[test]
fn test_diff_ref_sdk_parse_aes_key_base64_raw_and_hex() {
    let raw = [7_u8; 16];
    let raw_b64 = STANDARD.encode(raw);
    assert_eq!(parse_aes_key_base64(&raw_b64).expect("raw"), raw);

    let hex = bytes_to_hex(&raw);
    let hex_b64 = STANDARD.encode(hex);
    assert_eq!(parse_aes_key_base64(&hex_b64).expect("hex"), raw);
}

#[test]
fn test_diff_ref_sdk_aes_ecb_roundtrip() {
    let key = random_16_bytes();
    let plaintext = b"hello weixin reference parity";
    let encrypted = encrypt_aes_ecb(plaintext, &key).expect("encrypt");
    let decrypted = decrypt_aes_ecb(&encrypted, &key).expect("decrypt");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_diff_ref_sdk_cdn_url_builders() {
    assert_eq!(
        build_cdn_upload_url("https://novac2c.cdn.weixin.qq.com/c2c", "a+b", "f/1"),
        "https://novac2c.cdn.weixin.qq.com/c2c/upload?encrypted_query_param=a%2Bb&filekey=f%2F1"
    );
    assert_eq!(
        build_cdn_download_url("https://novac2c.cdn.weixin.qq.com/c2c", "abc=="),
        "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_query_param=abc%3D%3D"
    );
}

#[test]
fn test_diff_ref_sdk_get_upload_url_body_shape() {
    let body = build_get_upload_url_body("file-key", "u@im.wechat", 1, b"abc", 16, "001122");
    assert_eq!(body["filekey"], "file-key");
    assert_eq!(body["media_type"], 1);
    assert_eq!(body["to_user_id"], "u@im.wechat");
    assert_eq!(body["rawsize"], 3);
    assert_eq!(body["rawfilemd5"], "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(body["filesize"], 16);
    assert_eq!(body["no_need_thumb"], true);
    assert_eq!(body["aeskey"], "001122");
    assert!(body["base_info"]["channel_version"].as_str().is_some());
}

#[test]
fn test_diff_ref_sdk_send_media_message_body_shape() {
    let uploaded = UploadedWeixinMedia {
        download_encrypted_query_param: "enc-param".to_owned(),
        aes_key_raw: [9_u8; 16],
        plaintext_size: 20,
        ciphertext_size: 32,
        media_type: 1,
        file_name: None,
    };
    let body = build_send_media_message_body("u@im.wechat", "ctx-token", "client-id-1", &uploaded);
    assert_eq!(body["msg"]["to_user_id"], "u@im.wechat");
    assert_eq!(body["msg"]["context_token"], "ctx-token");
    assert_eq!(
        body["msg"]["item_list"].as_array().map(|v| v.len()),
        Some(1)
    );
    assert_eq!(body["msg"]["item_list"][0]["type"], 2);
    assert_eq!(
        body["msg"]["item_list"][0]["image_item"]["media"]["encrypt_query_param"],
        "enc-param"
    );
    assert_eq!(
        body["msg"]["item_list"][0]["image_item"]["media"]["encrypt_type"],
        1
    );
    let expected_aes_key = STANDARD.encode(bytes_to_hex(&uploaded.aes_key_raw).as_bytes());
    assert_eq!(
        body["msg"]["item_list"][0]["image_item"]["media"]["aes_key"],
        expected_aes_key
    );
    assert_eq!(body["msg"]["item_list"][0]["image_item"]["mid_size"], 32);
}

#[test]
fn test_diff_ref_sdk_send_video_message_body_shape() {
    let uploaded = UploadedWeixinMedia {
        download_encrypted_query_param: "enc-param".to_owned(),
        aes_key_raw: [5_u8; 16],
        plaintext_size: 99,
        ciphertext_size: 112,
        media_type: 2,
        file_name: None,
    };
    let body = build_send_media_message_body("u@im.wechat", "ctx-token", "client-id-2", &uploaded);
    assert_eq!(body["msg"]["item_list"][0]["type"], 5);
    assert_eq!(body["msg"]["item_list"][0]["video_item"]["video_size"], 112);
}

#[test]
fn test_diff_ref_sdk_send_file_message_body_shape() {
    let uploaded = UploadedWeixinMedia {
        download_encrypted_query_param: "enc-param".to_owned(),
        aes_key_raw: [6_u8; 16],
        plaintext_size: 77,
        ciphertext_size: 80,
        media_type: 3,
        file_name: None,
    };
    let body = build_send_media_message_body("u@im.wechat", "ctx-token", "client-id-3", &uploaded);
    assert_eq!(body["msg"]["item_list"][0]["type"], 4);
    assert_eq!(body["msg"]["item_list"][0]["file_item"]["len"], "77");
    // Regression: when `file_name` is None, fall back to the historical
    // placeholder so existing behaviour is preserved.
    assert_eq!(
        body["msg"]["item_list"][0]["file_item"]["file_name"],
        "attachment.bin"
    );

    // New: when the uploader supplies the original filename, WeChat should
    // receive exactly that value (this is the bug fix for PDFs arriving as
    // attachment.bin on mobile).
    let original_file_name = "resume.pdf";
    let uploaded_named = UploadedWeixinMedia {
        file_name: Some(original_file_name.to_owned()),
        ..uploaded
    };
    let body_named =
        build_send_media_message_body("u@im.wechat", "ctx-token", "client-id-3", &uploaded_named);
    assert_eq!(
        body_named["msg"]["item_list"][0]["file_item"]["file_name"],
        original_file_name
    );

    // Whitespace-only filenames are treated as absent so clients never see
    // a blank bubble title.
    let uploaded_blank = UploadedWeixinMedia {
        file_name: Some("   ".to_owned()),
        ..uploaded_named
    };
    let body_blank =
        build_send_media_message_body("u@im.wechat", "ctx-token", "client-id-3", &uploaded_blank);
    assert_eq!(
        body_blank["msg"]["item_list"][0]["file_item"]["file_name"],
        "attachment.bin"
    );
}

#[test]
fn test_classify_media_type_from_url() {
    assert_eq!(classify_media_type_from_url("https://a/b/c.mp4"), 2);
    assert_eq!(classify_media_type_from_url("https://a/b/c.png"), 1);
    assert_eq!(classify_media_type_from_url("/tmp/c.png?x=1"), 1);
    assert_eq!(classify_media_type_from_url("https://a/b/c.pdf"), 3);
}

#[test]
fn test_diff_ref_sdk_get_config_body_shape() {
    let body = build_get_config_body("u@im.wechat", Some("ctx-token"));
    assert_eq!(body["ilink_user_id"], "u@im.wechat");
    assert_eq!(body["context_token"], "ctx-token");
    assert!(body["base_info"]["channel_version"].as_str().is_some());
}

#[test]
fn test_diff_ref_sdk_send_typing_body_shape() {
    let body = build_send_typing_body("u@im.wechat", "typing-ticket", 2);
    assert_eq!(body["ilink_user_id"], "u@im.wechat");
    assert_eq!(body["typing_ticket"], "typing-ticket");
    assert_eq!(body["status"], 2);
}

#[tokio::test]
async fn test_upload_media_to_cdn_pipeline_with_mocks() {
    let api = MockServer::start().await;
    let cdn = MockServer::start().await;
    let account = WeixinAccount {
        token: "token-1".to_owned(),
        uin: String::new(),
        enabled: true,
        base_url: api.uri(),
        name: None,
        agent_id: "claude".to_owned(),
        workspace_dir: None,
        streaming_update: true,
    };

    Mock::given(method("POST"))
        .and(path("/ilink/bot/getuploadurl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ret": 0,
            "upload_param": "up-param"
        })))
        .mount(&api)
        .await;

    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-encrypted-param", "dl-param"))
        .mount(&cdn)
        .await;

    let client = Client::new();
    let uploaded = {
        // Override channel CDN constant path by constructing URL directly through helper.
        let plaintext = b"hello media";
        let aes_key_raw = random_16_bytes();
        let aes_key_hex = bytes_to_hex(&aes_key_raw);
        let ciphertext_size = aes_ecb_padded_size(plaintext.len());
        let filekey = bytes_to_hex(uuid::Uuid::new_v4().as_bytes());
        let upload_result = get_upload_url(
            &client,
            &account,
            "u@im.wechat",
            1,
            &filekey,
            plaintext,
            ciphertext_size,
            &aes_key_hex,
        )
        .await
        .expect("get upload url");
        let upload_param = match &upload_result {
            UploadUrlResult::FullUrl(u) => u.clone(),
            UploadUrlResult::Param(p) => p.clone(),
        };
        let ciphertext = encrypt_aes_ecb(plaintext, &aes_key_raw).expect("encrypt");
        let url = build_cdn_upload_url(&cdn.uri(), &upload_param, &filekey);
        let response = client
            .post(url)
            .header("Content-Type", "application/octet-stream")
            .body(ciphertext)
            .send()
            .await
            .expect("cdn upload");
        assert!(response.status().is_success());
        let download_param = response
            .headers()
            .get("x-encrypted-param")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        UploadedWeixinMedia {
            download_encrypted_query_param: download_param,
            aes_key_raw,
            plaintext_size: plaintext.len(),
            ciphertext_size,
            media_type: 1,
            file_name: None,
        }
    };

    assert_eq!(uploaded.download_encrypted_query_param, "dl-param");
    assert_eq!(uploaded.media_type, 1);
    assert!(uploaded.ciphertext_size >= uploaded.plaintext_size);
}

#[tokio::test]
async fn test_send_media_and_typing_requests_shape_with_mocks() {
    let api = MockServer::start().await;
    let account = weixin_test_account(&api, "token-2");

    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/getconfig"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ret":0,"typing_ticket":"ticket-1"})),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendtyping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
        .mount(&api)
        .await;

    let uploaded = UploadedWeixinMedia {
        download_encrypted_query_param: "dl-p".to_owned(),
        aes_key_raw: [3_u8; 16],
        plaintext_size: 10,
        ciphertext_size: 16,
        media_type: 1,
        file_name: None,
    };
    let message_id = send_media_message(
        &Client::new(),
        &account,
        "u@im.wechat",
        &uploaded,
        "caption",
        Some("ctx-1"),
    )
    .await
    .expect("send media message");
    assert!(!message_id.is_empty());

    let ticket = fetch_typing_ticket(&Client::new(), &account, "u@im.wechat", Some("ctx-1"))
        .await
        .expect("fetch typing ticket");
    assert_eq!(ticket.as_deref(), Some("ticket-1"));
    send_typing_status(&Client::new(), &account, "u@im.wechat", "ticket-1", 1)
        .await
        .expect("typing status");

    let requests = api.received_requests().await.expect("requests");
    let has_sendmessage = requests
        .iter()
        .any(|req| req.url.path() == "/ilink/bot/sendmessage");
    let has_getconfig = requests
        .iter()
        .any(|req| req.url.path() == "/ilink/bot/getconfig");
    let has_sendtyping = requests
        .iter()
        .any(|req| req.url.path() == "/ilink/bot/sendtyping");
    assert!(has_sendmessage && has_getconfig && has_sendtyping);
    let sendmessage_requests: Vec<_> = requests
        .iter()
        .filter(|req| req.url.path() == "/ilink/bot/sendmessage")
        .collect();
    assert_eq!(sendmessage_requests.len(), 2);
    for req in sendmessage_requests {
        let body: Value = serde_json::from_slice(&req.body).expect("sendmessage json body");
        assert_eq!(
            body["msg"]["item_list"].as_array().map(|v| v.len()),
            Some(1)
        );
    }
}

#[tokio::test]
async fn test_send_text_message_with_state_reuses_client_id_for_updates() {
    let api = MockServer::start().await;
    let account = weixin_test_account(&api, "token-state");
    let token = format!("ctx-state-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;

    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
        .mount(&api)
        .await;

    let client_id = format!("client-{}", uuid::Uuid::new_v4());
    for state in [1_u8, 1, 1, 2] {
        send_text_message_with_state(
            &Client::new(),
            &account,
            "u@im.wechat",
            &format!("state-{state}"),
            Some(&token),
            &client_id,
            state,
        )
        .await
        .expect("send text update");
    }

    let requests = api.received_requests().await.expect("requests");
    let sendmessage_requests = requests
        .iter()
        .filter(|req| req.url.path() == "/ilink/bot/sendmessage")
        .collect::<Vec<_>>();
    assert_eq!(sendmessage_requests.len(), 4);
    let states = sendmessage_requests
        .iter()
        .map(|req| {
            let body: Value = serde_json::from_slice(&req.body).expect("json body");
            assert_eq!(body["msg"]["client_id"], client_id);
            body["msg"]["message_state"].as_u64().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(states, vec![1, 1, 1, 2]);
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_live_message_short_answer_fast_path_stays_pristine_until_finish() {
    let token = format!("ctx-short-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;
    let mut live = LiveMessage::open(token.clone()).await;
    let now = Instant::now();

    live.append_delta("ok", &HashSet::new(), now);

    assert_eq!(live.state, LiveMessageState::Pristine);
    assert_eq!(live.text_visible, "ok");
    assert!(!live.should_send_generating(now).await);
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_live_message_extracts_split_markdown_image_once() {
    let token = format!("ctx-image-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;
    let mut live = LiveMessage::open(token.clone()).await;
    let sent = HashSet::new();
    let now = Instant::now();

    live.append_delta("see ![alt](file://", &sent, now);
    assert!(live.pending_media_refs.is_empty());
    live.append_delta("/tmp/example.png)", &sent, now);
    live.append_delta(" again", &sent, now);

    assert_eq!(live.pending_media_refs.len(), 1);
    assert!(matches!(
        &live.pending_media_refs[0],
        OutboundMediaRef::LocalPath(path) if path == "/tmp/example.png"
    ));
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_live_message_budget_reserves_final_finish() {
    let token = format!("ctx-reserve-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;
    for _ in 0..(TOKEN_SEND_LIMIT - 1) {
        token_send_increment(&token).await;
    }
    let mut live = LiveMessage::open(token.clone()).await;
    live.state = LiveMessageState::Updating;
    live.text_visible = "long enough text.".to_owned();

    assert_eq!(
        live.needs_budget_finalize().await,
        Some(FinalizeReason::BudgetToken)
    );
    assert!(!live.should_send_generating(Instant::now()).await);
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_live_message_per_client_id_budget_caps_generating_updates() {
    let token = format!("ctx-live-cap-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;
    let mut live = LiveMessage::open(token.clone()).await;
    live.state = LiveMessageState::Updating;
    live.sends_used = LIVE_MESSAGE_MAX_GENERATING_SENDS;

    assert_eq!(
        live.needs_budget_finalize().await,
        Some(FinalizeReason::BudgetMessage)
    );
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_live_message_suppresses_generating_for_unterminated_image_ref() {
    let token = format!("ctx-tail-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;
    let mut live = LiveMessage::open(token.clone()).await;
    let now = Instant::now();

    live.append_delta(
        "This is long enough ![partial](file:///tmp/example",
        &HashSet::new(),
        now,
    );

    assert!(live.has_unterminated_markdown_image_ref_tail());
    assert!(!live.should_send_generating(now).await);
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_streaming_update_consumer_user_ack_finalizes_visible_and_uses_fresh_token() {
    let api = MockServer::start().await;
    let account = weixin_test_account(&api, "token-stream-ack");
    let account_id = format!("acct-{}", uuid::Uuid::new_v4());
    let user_id = "u@im.wechat".to_owned();
    let original_token = format!("ctx-original-{}", uuid::Uuid::new_v4());
    let fresh_token = format!("ctx-fresh-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&original_token).await;
    token_send_count_reset(&fresh_token).await;

    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
        .mount(&api)
        .await;

    // Simulate the follow-up inbound message refreshing the stored token before
    // the active provider stream emits its UserAck boundary.
    set_context_token(&account_id, &user_id, &fresh_token).await;

    let router = Arc::new(Mutex::new(MessageRouter::new(
        Arc::new(garyx_router::InMemoryThreadStore::new()),
        garyx_models::config::GaryxConfig::default(),
    )));
    let ctx = WeixinStreamConsumerContext {
        http: Client::new(),
        account,
        account_id: account_id.clone(),
        user_id: user_id.clone(),
        context_token: original_token.clone(),
        router,
        thread_id: Arc::new(std::sync::Mutex::new(String::new())),
        typing_ticket: None,
        running: Arc::new(AtomicBool::new(true)),
    };
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (stream_done_tx, stream_done_rx) = oneshot::channel();
    let final_done_flush_sent = Arc::new(AtomicBool::new(false));
    let seen_done_event = Arc::new(AtomicBool::new(false));

    tokio::spawn(run_streaming_update_consumer(
        ctx,
        event_rx,
        stream_done_tx,
        final_done_flush_sent.clone(),
        seen_done_event.clone(),
    ));

    event_tx
        .send(StreamEvent::Delta {
            text: "initial reply is visible.".to_owned(),
        })
        .expect("send initial delta");
    event_tx
        .send(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: Some("pending-1".to_owned()),
        })
        .expect("send user ack");
    event_tx
        .send(StreamEvent::Delta {
            text: "follow-up reply is visible.".to_owned(),
        })
        .expect("send follow-up delta");
    event_tx.send(StreamEvent::Done).expect("send done");

    tokio::time::timeout(Duration::from_secs(5), stream_done_rx)
        .await
        .expect("stream consumer timed out")
        .expect("stream consumer dropped done sender");

    assert!(seen_done_event.load(Ordering::Relaxed));
    assert!(final_done_flush_sent.load(Ordering::Relaxed));

    let requests = api.received_requests().await.expect("requests");
    let sendmessage_requests = requests
        .iter()
        .filter(|req| req.url.path() == "/ilink/bot/sendmessage")
        .collect::<Vec<_>>();
    assert_eq!(sendmessage_requests.len(), 4);

    let sent = sendmessage_requests
        .iter()
        .map(|req| {
            let body: Value = serde_json::from_slice(&req.body).expect("sendmessage json body");
            (
                body["msg"]["client_id"].as_str().unwrap().to_owned(),
                body["msg"]["message_state"].as_u64().unwrap(),
                body["msg"]["context_token"].as_str().unwrap().to_owned(),
                body["msg"]["item_list"][0]["text_item"]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        sent.iter()
            .map(|(_, state, _, _)| *state)
            .collect::<Vec<_>>(),
        vec![1, 2, 1, 2]
    );
    assert_eq!(sent[0].0, sent[1].0);
    assert_eq!(sent[2].0, sent[3].0);
    assert_ne!(sent[0].0, sent[2].0);
    assert_eq!(sent[0].2, original_token);
    assert_eq!(sent[1].2, original_token);
    assert_eq!(sent[2].2, fresh_token);
    assert_eq!(sent[3].2, fresh_token);
    assert_eq!(sent[0].3, "initial reply is visible.");
    assert_eq!(sent[1].3, "initial reply is visible.");
    assert_eq!(sent[2].3, "follow-up reply is visible.");
    assert_eq!(sent[3].3, "follow-up reply is visible.");

    token_send_count_reset(&original_token).await;
    token_send_count_reset(&fresh_token).await;
}

#[tokio::test]
async fn test_send_media_message_ret_minus_14_pauses_session() {
    let api = MockServer::start().await;
    let account = weixin_test_account(&api, "acct-media-pause:secret");
    let token = format!("ctx-media-pause-{}", uuid::Uuid::new_v4());
    token_send_count_reset(&token).await;

    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ret": -14,
            "errmsg": "expired"
        })))
        .mount(&api)
        .await;

    let uploaded = UploadedWeixinMedia {
        download_encrypted_query_param: "dl-p".to_owned(),
        aes_key_raw: [3_u8; 16],
        plaintext_size: 10,
        ciphertext_size: 16,
        media_type: 1,
        file_name: None,
    };
    let error = send_media_message(
        &Client::new(),
        &account,
        "u@im.wechat",
        &uploaded,
        "",
        Some(&token),
    )
    .await
    .expect_err("media send should fail");
    assert!(error.to_string().contains("ret=-14"));
    assert!(is_session_paused("acct-media-pause").await);
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_extract_cdn_non_image_media_metadata_skips_voice() {
    // Voice items (type 3) should be skipped entirely — SILK format
    // cannot be processed, and the transcribed text is already extracted
    // via voice_item.text in body_from_message_item.
    let item = WeixinMessageItem {
        r#type: 3,
        voice_item: Some(WeixinVoiceItem {
            text: "transcribed text".to_owned(),
            media: Some(WeixinCdnMedia {
                encrypt_query_param: "enc-q".to_owned(),
                aes_key: STANDARD.encode([7_u8; 16]),
                encrypt_type: 1,
            }),
        }),
        ..Default::default()
    };

    let metadata =
        extract_cdn_non_image_media_metadata(&Client::new(), "https://fake.cdn", &[item]).await;
    assert!(
        metadata.is_empty(),
        "voice items should be skipped, got: {metadata:?}"
    );
}

#[tokio::test]
async fn test_token_sends_remaining_returns_zero_at_limit() {
    // Use a unique token to avoid interference from other tests.
    let token = format!("test-exhausted-{}", uuid::Uuid::new_v4());

    // Initially the token should have TOKEN_SEND_LIMIT remaining.
    assert_eq!(token_sends_remaining(&token).await, TOKEN_SEND_LIMIT);

    // Exhaust the token by incrementing TOKEN_SEND_LIMIT times.
    for _ in 0..TOKEN_SEND_LIMIT {
        token_send_increment(&token).await;
    }

    // Now remaining should be 0.
    assert_eq!(token_sends_remaining(&token).await, 0);

    // Further increment should return None (limit reached).
    assert_eq!(token_send_increment(&token).await, None);

    // Clean up.
    token_send_count_reset(&token).await;
}

#[tokio::test]
async fn test_queue_preserves_media_in_text() {
    let account = "test-media-acct";
    let user = "test-media-user";

    // Queue a message that includes a media URL
    let text_with_media = "Hello\nhttps://example.com/image.png";
    queue_pending_outbound(account, user, text_with_media).await;

    let drained = drain_pending_outbound(account, user).await;
    assert_eq!(drained.len(), 1);
    assert!(drained[0].text.contains("https://example.com/image.png"));
    assert!(drained[0].text.contains("Hello"));
}

#[tokio::test]
async fn test_pending_queue_merge_produces_single_text() {
    let account = "test-merge-acct";
    let user = "test-merge-user";

    queue_pending_outbound(account, user, "First message").await;
    queue_pending_outbound(account, user, "Second message").await;
    queue_pending_outbound(account, user, "Third message").await;

    let drained = drain_pending_outbound(account, user).await;
    assert_eq!(drained.len(), 3);

    // Simulate the merge logic from the flush code
    let merged = drained
        .iter()
        .map(|q| q.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    assert!(merged.contains("First message"));
    assert!(merged.contains("Second message"));
    assert!(merged.contains("Third message"));
    assert_eq!(merged.matches("\n\n").count(), 2);
}
