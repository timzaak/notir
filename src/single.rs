use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;

use salvo::prelude::*;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};

use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{FutureExt, StreamExt};
use nanoid::nanoid;
use salvo::http::Mime;
use salvo::http::headers::ContentType;
use serde::Deserialize;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::time::{Duration, interval, timeout};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Shot,
    PingPong,
}

type Users = RwLock<HashMap<String, mpsc::UnboundedSender<Result<Message, salvo::Error>>>>;
type CallbackChannels = DashMap<String, VecDeque<(String, oneshot::Sender<Bytes>)>>;

pub static ONLINE_USERS: LazyLock<Users> = LazyLock::new(Users::default);
pub static CALLBACK_CHANNELS: LazyLock<CallbackChannels> = LazyLock::new(CallbackChannels::default);

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
    tracing::info!("new chat user: {}", my_id);

    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);
    let fut = rx.forward(user_ws_tx).map(|_result| {
        // if let Err(e) = result {
        //    tracing::error!(error = e, "websocket send error");
        // }
    });
    tokio::task::spawn(fut);
    let my_id_clone_for_task = my_id.clone();
    let tx_clone_for_ping = tx.clone();
    let my_id_clone_for_ping = my_id.clone();

    let ping_task = async move {
        let mut ping_interval = interval(Duration::from_secs(30));
        ping_interval.tick().await; //jump first trigger

        loop {
            ping_interval.tick().await;
            if tx_clone_for_ping.send(Ok(Message::ping(vec![]))).is_err() {
                tracing::debug!(
                    "Failed to send ping to user {my_id_clone_for_ping}, connection likely closed"
                );
                break;
            }
            tracing::debug!("Sent ping to user: {my_id_clone_for_ping}");
        }
    };
    tokio::task::spawn(ping_task);

    let fut = async move {
        ONLINE_USERS
            .write()
            .await
            .insert(my_id_clone_for_task.clone(), tx);

        while let Some(result) = user_ws_rx.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_pong() {
                        tracing::debug!(
                            "Received pong from user: {}, ignoring",
                            my_id_clone_for_task
                        );
                        continue;
                    }

                    let data: Bytes = msg.as_bytes().to_vec().into();
                    if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&my_id_clone_for_task) {
                        if let Some((_id, tx)) = entry.pop_front() {
                            if let Err(e) = tx.send(data) {
                                tracing::error!(
                                    "Failed to send message to callback channel for user {my_id_clone_for_task}: {e:?}"
                                );
                            }
                        }
                    }
                }
                Err(_e) => {
                    break;
                }
            };
        }

        user_disconnected(my_id_clone_for_task).await;
    };
    tokio::task::spawn(fut);
}

pub async fn user_disconnected(my_id: String) {
    tracing::info!("good bye user: {}", my_id);
    ONLINE_USERS.write().await.remove(&my_id);
    CALLBACK_CHANNELS.remove(&my_id);
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

    let content_type = req
        .content_type()
        .unwrap_or_else(|| Mime::from(ContentType::octet_stream()));
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
            let users_map = ONLINE_USERS.read().await;
            if let Some(user_tx) = users_map.get(&string_uid) {
                let content_type_str = content_type.to_string();
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

                if user_tx.send(Ok(msg)).is_err() {
                    drop(users_map);
                    ONLINE_USERS.write().await.remove(&string_uid);
                    res.status_code(StatusCode::NOT_FOUND);
                    res.body("User disconnected during send");
                } else {
                    res.status_code(StatusCode::OK);
                }
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.body("User ID not found");
            }
        }
        Mode::PingPong => {
            let id = nanoid!();
            let (tx, rx) = oneshot::channel();
            let users_map = ONLINE_USERS.read().await;
            if let Some(user_tx) = users_map.get(&string_uid) {
                let content_type_str = content_type.to_string();
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
                if user_tx.send(Ok(msg)).is_err() {
                    drop(users_map);
                    ONLINE_USERS.write().await.remove(&string_uid);
                    let _ = CALLBACK_CHANNELS
                        .entry(string_uid.clone())
                        .or_default()
                        .pop_front();
                    res.status_code(StatusCode::NOT_FOUND);
                    res.body("User disconnected during send");
                    return;
                }
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.body("User ID not found");
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
