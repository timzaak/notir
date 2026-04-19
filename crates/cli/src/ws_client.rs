use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::args::SubscriptionMode;
use crate::js_runtime::JsEngine;
use crate::output::OutputWriter;
use crate::script_api::WsEvent;

pub async fn run_client(
    server: &str,
    id: &str,
    mode: SubscriptionMode,
    js_engine: &JsEngine,
    writer: &mut OutputWriter,
) -> Result<()> {
    let url = format!("{}/{}/sub?id={}", server.trim_end_matches('/'), mode, id);

    tracing::info!("Connecting to {}", url);

    let (mut ws_stream, _) = connect_async(&url)
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;

    tracing::info!("Connected to {}", url);

    let source = mode.to_string();

    while let Some(msg) = ws_stream.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let event = WsEvent::from_text(text.to_string(), &source);
                handle_event(&event, js_engine, writer)?;
            }
            Ok(Message::Binary(data)) => {
                let event = WsEvent::from_binary(&data, &source);
                handle_event(&event, js_engine, writer)?;
            }
            Ok(Message::Ping(_)) => {
                tracing::debug!("Received ping, auto-responding with pong");
            }
            Ok(Message::Pong(_)) => {
                tracing::debug!("Received pong");
            }
            Ok(Message::Close(frame)) => {
                tracing::info!("Server closed connection: {:?}", frame);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("WebSocket error: {}", e);
                break;
            }
        }
    }

    let _ = ws_stream.close(None).await;
    Ok(())
}

fn handle_event(event: &WsEvent, js_engine: &JsEngine, writer: &mut OutputWriter) -> Result<()> {
    let event_json = serde_json::to_string(event)?;
    match js_engine.transform(&event_json) {
        Ok(Some(output)) => {
            writer.write_message(&output)?;
        }
        Ok(None) => {
            tracing::debug!("Message discarded by transform script");
        }
        Err(e) => {
            tracing::warn!("JS transform error: {}, outputting raw message", e);
            writer.write_message(&transform_error_output(event))?;
        }
    }
    Ok(())
}

fn transform_error_output(event: &WsEvent) -> String {
    format!("[TRANSFORM_ERROR] {}", event.raw_output())
}

#[cfg(test)]
mod tests {
    use super::transform_error_output;
    use crate::script_api::WsEvent;

    #[test]
    fn transform_error_output_keeps_binary_payload() {
        let event = WsEvent::from_binary(&[0xca, 0xfe], "single");
        assert_eq!(transform_error_output(&event), "[TRANSFORM_ERROR] cafe");
    }
}
