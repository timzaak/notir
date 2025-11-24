use std::collections::VecDeque;
use std::sync::LazyLock;

use salvo::prelude::*;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};

use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{FutureExt, StreamExt};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, interval, timeout};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Shot,
    PingPong,
}

type Users = DashMap<String, DashMap<String, mpsc::UnboundedSender<Result<Message, salvo::Error>>>>;
type CallbackChannels = DashMap<String, VecDeque<(String, oneshot::Sender<Bytes>)>>;

pub static ONLINE_USERS: LazyLock<Users> = LazyLock::new(Users::default);
pub static CALLBACK_CHANNELS: LazyLock<CallbackChannels> = LazyLock::new(CallbackChannels::default);

#[derive(Serialize)]
struct ConnectionCount {
    count: usize,
}

#[handler]
pub async fn connections(req: &mut Request, res: &mut Response) {
    let string_uid = req.query::<String>("id").unwrap_or_default();
    if string_uid.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        return;
    }
    let count = ONLINE_USERS
        .get(&string_uid)
        .map(|conns| conns.len())
        .unwrap_or(0);
    res.render(Json(ConnectionCount { count }));
}

#[handler]
pub async fn user_connected(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
    let string_uid = req
        .query::<String>("id")
        .ok_or_else(|| StatusError::bad_request().detail("Missing 'id' query parameter"))?;
    if string_uid.is_empty() {
        return Err(StatusError::bad_request().detail("'id' query parameter cannot be empty"));
    }
    WebSocketUpgrade::new()
        .upgrade(req, res, |ws| handle_socket(ws, string_uid))
        .await
}

async fn handle_socket(ws: WebSocket, my_id: String) {
    tracing::info!("new single user: {}", my_id);
    let conn_id = nanoid!();

    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            tracing::error!(error = ?e, "websocket send error");
        }
    }));

    let my_id_clone = my_id.clone();
    let conn_id_clone = conn_id.clone();
    let tx_clone = tx.clone();
    let ping_task = async move {
        let mut ping_interval = interval(Duration::from_secs(30));
        ping_interval.tick().await;

        loop {
            ping_interval.tick().await;
            if tx_clone.send(Ok(Message::ping(vec![]))).is_err() {
                tracing::debug!(
                    "Failed to send ping to user {}, connection {}, likely closed",
                    my_id_clone,
                    conn_id_clone
                );
                break;
            }
            tracing::debug!(
                "Sent ping to user: {}, connection: {}",
                my_id_clone,
                conn_id_clone
            );
        }
    };
    tokio::task::spawn(ping_task);

    ONLINE_USERS
        .entry(my_id.clone())
        .or_default()
        .insert(conn_id.clone(), tx);
    while let Some(result) = user_ws_rx.next().await {
        match result {
            Ok(msg) => {
                if msg.is_pong() {
                    tracing::debug!("Received pong from user: {}, ignoring", my_id);
                    continue;
                }
                let data: Bytes = msg.as_bytes().to_vec().into();
                if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&my_id)
                    && let Some((_id, tx)) = entry.pop_front()
                    && let Err(e) = tx.send(data)
                {
                    tracing::error!(
                        "Failed to send message to callback channel for user {}: {:?}",
                        my_id,
                        e
                    );
                }
            }
            Err(e) => {
                tracing::warn!("WebSocket error for user {}: {:?}", my_id, e);
                break;
            }
        };
    }

    user_disconnected(my_id, conn_id).await;
}

pub async fn user_disconnected(my_id: String, conn_id: String) {
    tracing::info!("subscriber disconnected: user {}, conn {}", my_id, conn_id);
    if let Some(user_conns) = ONLINE_USERS.get_mut(&my_id) {
        user_conns.remove(&conn_id);
        if user_conns.is_empty() {
            drop(user_conns);
            ONLINE_USERS.remove(&my_id);
            CALLBACK_CHANNELS.remove(&my_id);
        }
    }
}

#[handler]
pub async fn publish_message(req: &mut Request, res: &mut Response) {
    let string_uid = req.query::<String>("id").unwrap_or_default();
    if string_uid.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.body("Missing 'id' query parameter for /pub");
        return;
    }
    let mode = req.query::<Mode>("mode").unwrap_or_default();

    let content_type_str = req
        .content_type()
        .map(|ct| ct.to_string())
        .unwrap_or_default();
    let body_bytes = match req.payload().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read payload for /pub: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.body("Failed to read request body");
            return;
        }
    };

    match mode {
        Mode::Shot => {
            if let Some(user_conns) = ONLINE_USERS.get(&string_uid) {
                let msg = if content_type_str.starts_with("application/json")
                    || content_type_str.starts_with("text/")
                {
                    match String::from_utf8(body_bytes.to_vec()) {
                        Ok(text_payload) => Message::text(text_payload),
                        Err(_) => {
                            res.status_code(StatusCode::BAD_REQUEST);
                            res.body("Invalid UTF-8 in body");
                            return;
                        }
                    }
                } else {
                    Message::binary(body_bytes.to_vec())
                };

                let mut disconnected_conns = Vec::new();
                for conn in user_conns.iter() {
                    if conn.value().send(Ok(msg.clone())).is_err() {
                        disconnected_conns.push(conn.key().clone());
                    }
                }

                if !disconnected_conns.is_empty() {
                    for conn_id in disconnected_conns {
                        user_conns.remove(&conn_id);
                    }
                    if user_conns.is_empty() {
                        ONLINE_USERS.remove(&string_uid);
                    }
                }
                res.status_code(StatusCode::OK);
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.body("subscriber id not found");
            }
        }
        Mode::PingPong => {
            let id = nanoid!();
            let (tx, rx) = oneshot::channel();
            if let Some(user_conns) = ONLINE_USERS.get(&string_uid) {
                let msg = if content_type_str.starts_with("application/json")
                    || content_type_str.starts_with("text/")
                {
                    match String::from_utf8(body_bytes.to_vec()) {
                        Ok(text_payload) => Message::text(text_payload),
                        Err(_) => {
                            res.status_code(StatusCode::BAD_REQUEST);
                            res.body("Invalid UTF-8 in body");
                            return;
                        }
                    }
                } else {
                    Message::binary(body_bytes.to_vec())
                };

                CALLBACK_CHANNELS
                    .entry(string_uid.clone())
                    .or_default()
                    .push_back((id.clone(), tx));

                let mut disconnected_conns = Vec::new();
                let mut sent = false;
                for conn in user_conns.iter() {
                    if conn.value().send(Ok(msg.clone())).is_ok() {
                        sent = true;
                        break;
                    } else {
                        disconnected_conns.push(conn.key().clone());
                    }
                }
                if !disconnected_conns.is_empty() {
                    for conn_id in disconnected_conns {
                        user_conns.remove(&conn_id);
                    }
                    if user_conns.is_empty() {
                        ONLINE_USERS.remove(&string_uid);
                    }
                }
                if !sent {
                    res.status_code(StatusCode::NOT_FOUND);
                    res.body("subscriber disconnected during send");
                    return;
                }
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.body("subscriber id not found");
                return;
            }

            match timeout(Duration::from_secs(5), rx).await {
                Ok(Ok(response)) => {
                    res.headers_mut().insert(
                        salvo::http::header::CONTENT_TYPE,
                        "application/octet-stream".parse().unwrap(),
                    );
                    res.write_body(response).ok();
                }
                Ok(Err(_)) => {
                    res.status_code(StatusCode::NO_CONTENT);
                }
                Err(_) => {
                    if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&string_uid) {
                        entry.retain(|(callback_id, _)| callback_id != &id);
                    }
                    res.status_code(StatusCode::REQUEST_TIMEOUT);
                    res.body("Request timeout after 5 seconds");
                }
            }
        }
    }
}
