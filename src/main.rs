use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;

use rust_embed::RustEmbed;
use salvo::prelude::*;
use salvo::serve_static::static_embed;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};

use tracing_subscriber::EnvFilter;

use futures_util::{FutureExt, StreamExt};
use salvo::http::Mime;
use salvo::http::headers::ContentType;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;
use dashmap::DashMap;
use serde::Deserialize;
use bytes::Bytes;
use nanoid::nanoid;

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Shot,
    PingPong,
}

type Users = RwLock<HashMap<String, mpsc::UnboundedSender<Result<Message, salvo::Error>>>>;
type CallbackChannels = DashMap<String, VecDeque<(String, oneshot::Sender<Bytes>)>>;

static ONLINE_USERS: LazyLock<Users> = LazyLock::new(Users::default);
static CALLBACK_CHANNELS: LazyLock<CallbackChannels> = LazyLock::new(CallbackChannels::default);

const HEART_BEATE:&[u8] = "!".as_bytes();
#[handler]
async fn user_connected(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
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

    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    // Use an unbounded channel to handle buffering and flushing of messages
    // to the websocket...
    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);
    let fut = rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            tracing::error!(error = ?e, "websocket send error");
        }
    });
    tokio::task::spawn(fut);
    let my_id_clone_for_task = my_id.clone();
    let fut = async move {
        ONLINE_USERS
            .write()
            .await
            .insert(my_id_clone_for_task.clone(), tx);

        while let Some(result) = user_ws_rx.next().await {
            match result {
                Ok(msg) => {
                    let is_text = msg.is_text();
                    let data: Bytes = msg.as_bytes().to_vec().into();
                    if is_text && data == HEART_BEATE {
                        continue
                    }
                    if let Some(mut entry) = CALLBACK_CHANNELS.get_mut(&my_id_clone_for_task) {
                        if let Some((_id, tx)) = entry.pop_front() {
                            if let Err(e) = tx.send(data) {
                                tracing::error!("Failed to send message to callback channel for user {my_id_clone_for_task}: {e:?}");
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("websocket error(uid={}): {}", my_id_clone_for_task, e);
                    break;
                }
            };
        }

        user_disconnected(my_id_clone_for_task).await;
    };
    tokio::task::spawn(fut);
}

async fn user_disconnected(my_id: String) {
    tracing::info!("good bye user: {}", my_id);
    ONLINE_USERS.write().await.remove(&my_id);
    CALLBACK_CHANNELS.remove(&my_id);
}

#[handler]
async fn publish_message(req: &mut Request, res: &mut Response) {
    let string_uid = req.query::<String>("id").unwrap_or_default();
    if string_uid.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.body("Missing 'id' query parameter for /pub");
        return;
    }
    let mode = req.query::<Mode>("mode").unwrap_or_default();

    let content_type = req.content_type().unwrap_or_else(|| Mime::from(ContentType::octet_stream()));
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
                let msg = if content_type_str.starts_with("application/json") || content_type_str.starts_with("text/") {
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
            let (tx, rx) = oneshot::channel();
            let users_map = ONLINE_USERS.read().await;
            if let Some(user_tx) = users_map.get(&string_uid) {
                let content_type_str = content_type.to_string();
                let msg = if content_type_str.starts_with("application/json") || content_type_str.starts_with("text/") {
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
                let id = nanoid!();
                CALLBACK_CHANNELS.entry(string_uid.clone()).or_default().push_back((id, tx));
                if user_tx.send(Ok(msg)).is_err() {
                    drop(users_map);
                    ONLINE_USERS.write().await.remove(&string_uid);
                    let _ = CALLBACK_CHANNELS.entry(string_uid.clone()).or_default().pop_front();
                    res.status_code(StatusCode::NOT_FOUND);
                    res.body("User disconnected during send");
                    return;
                }
            } else {
                res.status_code(StatusCode::NOT_FOUND);
                res.body("User ID not found");
                return;
            }

            match rx.await {
                Ok(response) => {
                    res.headers_mut().insert(salvo::http::header::CONTENT_TYPE, "application/octet-stream".parse().unwrap());
                    res.write_body(response).ok();
                }
                Err(_) => {
                    res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                    res.body("Failed to receive response from websocket");
                }
            }
        }
    }
}

#[derive(RustEmbed)]
#[folder = "static"]
struct Assets;

#[handler]
async fn health(res: &mut Response) {
    res.status_code(StatusCode::OK);
}

#[handler]
async fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
#[tokio::main]
async fn main() {
    // Initialize logging subsystem
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Bind server to port 5800
    let acceptor = TcpListener::new("0.0.0.0:5800").bind().await;
    let static_files =
        Router::with_hoop(Compression::new().enable_gzip(CompressionLevel::Fastest))
            .path("{*path}").get(static_embed::<Assets>().fallback("index.html"));

    let router = Router::new()
        .push(Router::with_path("health").goal(health))
        .push(Router::with_path("version").goal(version))
        .push(Router::with_path("sub").goal(user_connected))
        .push(Router::with_path("pub").post(publish_message))
        .push(static_files);

    tracing::debug!("{:?}", router);
    println!("Notir server start, binding: {:?}", acceptor.local_addr().unwrap());

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
