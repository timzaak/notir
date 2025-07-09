use std::collections::HashMap;
use std::sync::LazyLock;

use rust_embed::RustEmbed;
use salvo::prelude::*;
use salvo::serve_static::static_embed;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};
use serde::{Deserialize, Serialize};

use tracing_subscriber::EnvFilter;

use futures_util::{FutureExt, StreamExt};
use salvo::http::headers::ContentType;
use salvo::http::Mime;
use tokio::sync::{RwLock, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;


type Users = RwLock<HashMap<String, mpsc::UnboundedSender<Result<Message, salvo::Error>>>>;

static ONLINE_USERS: LazyLock<Users> = LazyLock::new(Users::default);


#[handler]
async fn user_connected(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
    let string_uid = req.query::<String>("id").ok_or_else(|| {

        StatusError::bad_request().detail("Missing 'id' query parameter")
    })?;
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
        ONLINE_USERS.write().await.insert(my_id_clone_for_task.clone(), tx);

        while let Some(result) = user_ws_rx.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::error!("websocket error(uid={}): {}", my_id_clone_for_task, e);
                    break;
                }
            };
            user_message(my_id_clone_for_task.clone(), msg).await;
        }

        user_disconnected(my_id_clone_for_task).await;
    };
    tokio::task::spawn(fut);
}
async fn user_message(my_id: String, msg: Message) {
    let msg_str = if let Ok(s) = msg.as_str() {
        s
    } else {
        // If it's not a text message, we could handle binary messages here if needed
        // For now, we'll ignore non-text messages from clients in the chat context
        tracing::debug!("Received non-text message from user {}", my_id);
        return;
    };

    let new_msg = format!("<User#{}>: {}", my_id, msg_str);
    tracing::debug!("Broadcasting message: {}", new_msg);

    // New message from this user, send it to everyone else (except same uid)...
    // Iterate over a clone of the keys to avoid holding the read lock while sending
    let users_map = ONLINE_USERS.read().await;
    let all_uids: Vec<String> = users_map.keys().cloned().collect();

    for uid_key in all_uids {
        if my_id != uid_key {
            if let Some(tx) = users_map.get(&uid_key) {
                if let Err(_disconnected) = tx.send(Ok(Message::text(new_msg.clone()))) {
                    // The tx is disconnected, our `user_disconnected` code
                    // should be happening in another task, nothing more to
                    // do here.
                    tracing::warn!("Failed to send message to user {}, channel disconnected.", uid_key);
                }
            }
        }
    }
}

async fn user_disconnected(my_id: String) {
    tracing::info!("good bye user: {}", my_id);
    ONLINE_USERS.write().await.remove(&my_id);
}

#[handler]
async fn publish_message(req: &mut Request, res: &mut Response, ctrl: &mut FlowCtrl) {
    let string_uid = req.query::<String>("id").ok_or_else(|| {
        StatusError::bad_request().detail("Missing 'id' query parameter for /pub")
    });
    let string_uid = match string_uid {
        Ok(id) => {
            if id.is_empty() {
                res.render(StatusError::bad_request().detail("'id' query parameter cannot be empty for /pub"));
                return;
            }
            id
        }
        Err(_) => {
            res.render(StatusError::bad_request().detail("Missing 'id' query parameter for /pub"));
            return;
        }
    };
    let content_type = req.content_type().unwrap_or_else(|| Mime::from(ContentType::octet_stream()));
    let body_bytes = match req.payload().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read payload for /pub: {}", e);
            res.render(StatusError::internal_server_error().detail("Failed to read request body"));
            return;
        }
    };

    let users_map = ONLINE_USERS.read().await;
    if let Some(tx) = users_map.get(&string_uid) {
        let content_type = content_type.to_string();
        let msg = if content_type.starts_with("application/json") {

            match String::from_utf8(body_bytes.to_vec()) {
                Ok(text_payload) => Message::text(text_payload),
                Err(_) => Message::binary(body_bytes.to_owned()), // Fallback to binary if not valid UTF-8
            }
        } else if content_type.starts_with("text/") {
            match String::from_utf8(body_bytes.to_vec()) {
                Ok(text_payload) => Message::text(text_payload),
                Err(_) => {
                     // if text/* is not valid utf8, it's a bad request.
                    res.render(StatusError::bad_request().detail("Invalid UTF-8 in text body"));
                    return;
                }
            }
        }
        else {
            Message::binary(body_bytes.to_owned())
        };

        if tx.send(Ok(msg)).is_ok() {
            res.status_code(StatusCode::OK);
            //res.render("Message published");
        } else {
            drop(users_map); // Release read lock before acquiring write lock
            ONLINE_USERS.write().await.remove(&string_uid);
            res.status_code(StatusCode::NOT_FOUND);
            //res.render("User disconnected during send");
        }
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        //res.render("User ID not found");
    }
}

#[derive(RustEmbed)]
#[folder = "static"]
struct Assets;

#[handler]
async fn hello() -> &'static str {
    "Hello World"
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
        Router::with_path("{*path}").get(static_embed::<Assets>().fallback("index.html"));

    let router = Router::new()
        .push(Router::with_path("hello").goal(hello))
        .push(Router::with_path("sub").goal(user_connected))
        .push(Router::with_path("pub").post(publish_message))
        .push(static_files);

    tracing::debug!("{:?}", router);

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
