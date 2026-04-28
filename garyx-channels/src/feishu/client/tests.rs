use super::*;

#[test]
fn ws_endpoint_client_config_maps_reconnect_interval() {
    let endpoint = WsEndpointData {
        ws_url: "wss://example.test/ws?service_id=42".to_owned(),
        client_config: Some(WsClientConfig {
            ping_interval: 30,
            reconnect_interval: 11,
        }),
    };

    let ping_interval_secs = endpoint
        .client_config
        .as_ref()
        .map(|c| c.ping_interval)
        .unwrap_or(120);
    let reconnect_interval_secs = endpoint
        .client_config
        .as_ref()
        .map(|c| c.reconnect_interval)
        .filter(|secs| *secs > 0)
        .unwrap_or(WS_RECONNECT_DELAY.as_secs());

    assert_eq!(ping_interval_secs, 30);
    assert_eq!(reconnect_interval_secs, 11);
}

#[test]
fn ws_endpoint_client_config_uses_default_reconnect_interval_when_zero() {
    let endpoint = WsEndpointData {
        ws_url: "wss://example.test/ws?service_id=1".to_owned(),
        client_config: Some(WsClientConfig {
            ping_interval: 30,
            reconnect_interval: 0,
        }),
    };

    let reconnect_interval_secs = endpoint
        .client_config
        .as_ref()
        .map(|c| c.reconnect_interval)
        .filter(|secs| *secs > 0)
        .unwrap_or(WS_RECONNECT_DELAY.as_secs());

    assert_eq!(reconnect_interval_secs, WS_RECONNECT_DELAY.as_secs());
}
