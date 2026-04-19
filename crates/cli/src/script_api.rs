use serde::Serialize;

/// Event object passed into the user's JS `transform()` function.
#[derive(Debug, Serialize)]
pub struct WsEvent {
    /// UTF-8 text content (null if binary message)
    pub text: Option<String>,
    /// Hex-encoded binary content (null if text message)
    pub binary: Option<String>,
    /// ISO 8601 timestamp of when the message was received
    pub timestamp: String,
    /// "text" or "binary"
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    /// "single" or "broad"
    pub source: String,
}

fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_default()
}

fn to_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

impl WsEvent {
    pub fn from_text(text: String, source: &str) -> Self {
        Self {
            text: Some(text),
            binary: None,
            timestamp: now_iso8601(),
            msg_type: "text",
            source: source.to_string(),
        }
    }

    pub fn from_binary(data: &[u8], source: &str) -> Self {
        Self {
            text: None,
            binary: Some(to_hex(data)),
            timestamp: now_iso8601(),
            msg_type: "binary",
            source: source.to_string(),
        }
    }

    pub fn raw_output(&self) -> &str {
        self.text
            .as_deref()
            .or(self.binary.as_deref())
            .unwrap_or_default()
    }
}

/// Built-in passthrough script used when --script is not provided.
pub(crate) const PASSTHROUGH_SCRIPT: &str =
    "function transform(e) { return e.text !== null ? e.text : e.binary; }";

#[cfg(test)]
mod tests {
    use super::WsEvent;

    #[test]
    fn raw_output_prefers_text() {
        let event = WsEvent::from_text("hello".to_string(), "single");
        assert_eq!(event.raw_output(), "hello");
    }

    #[test]
    fn raw_output_uses_binary_hex() {
        let event = WsEvent::from_binary(&[0xde, 0xad, 0xbe, 0xef], "broad");
        assert_eq!(event.raw_output(), "deadbeef");
    }
}
