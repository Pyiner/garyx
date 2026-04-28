//! Feishu WebSocket binary protocol (pbbp2).
//!
//! The Feishu long-connection SDK uses protobuf-encoded binary frames over
//! WebSocket rather than plain JSON text messages.

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct Header {
    #[prost(string, required, tag = "1")]
    pub key: String,
    #[prost(string, required, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Frame {
    #[prost(uint64, required, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    pub log_id: u64,
    #[prost(int32, required, tag = "3")]
    pub service: i32,
    #[prost(int32, required, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

// Frame method values
pub const METHOD_CONTROL: i32 = 0;
pub const METHOD_DATA: i32 = 1;

// Header key constants
pub const HEADER_TYPE: &str = "type";
pub const HEADER_BIZ_RT: &str = "biz_rt";

// Message type values
pub const MSG_TYPE_EVENT: &str = "event";
pub const MSG_TYPE_PING: &str = "ping";
pub const MSG_TYPE_PONG: &str = "pong";

impl Frame {
    pub fn header_value(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
    }

    pub fn payload_str(&self) -> Option<&str> {
        self.payload
            .as_deref()
            .and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Build a ping frame.
    pub fn ping(service_id: i32) -> Self {
        Frame {
            seq_id: 0,
            log_id: 0,
            service: service_id,
            method: METHOD_CONTROL,
            headers: vec![Header {
                key: HEADER_TYPE.to_owned(),
                value: MSG_TYPE_PING.to_owned(),
            }],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        }
    }

    /// Build an event acknowledgement frame (response to an event).
    pub fn event_ack(original: &Frame, biz_rt_ms: i64) -> Self {
        let mut headers = original.headers.clone();
        headers.push(Header {
            key: HEADER_BIZ_RT.to_owned(),
            value: biz_rt_ms.to_string(),
        });

        let ack_payload = serde_json::json!({ "code": 200 });
        Frame {
            seq_id: original.seq_id,
            log_id: original.log_id,
            service: original.service,
            method: original.method,
            headers,
            payload_encoding: original.payload_encoding.clone(),
            payload_type: original.payload_type.clone(),
            payload: Some(ack_payload.to_string().into_bytes()),
            log_id_new: original.log_id_new.clone(),
        }
    }
}
