use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use rust_embed::RustEmbed;
use salvo::prelude::*;
use salvo::serve_static::static_embed;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};
use serde::{Deserialize, Serialize};

use tracing_subscriber::EnvFilter;

use futures_util::{FutureExt, StreamExt};
use tokio::sync::{RwLock, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;


type Users = RwLock<HashMap<usize, mpsc::UnboundedSender<Result<Message, salvo::Error>>>>;

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static ONLINE_USERS: LazyLock<Users> = LazyLock::new(Users::default);


#[handler]
async fn user_connected(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
    WebSocketUpgrade::new()
        .upgrade(req, res, handle_socket)
        .await
}
async fn handle_socket(ws: WebSocket) {
    // Use a counter to assign a new unique ID for this user.
    let my_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);

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
    let fut = async move {
        ONLINE_USERS.write().await.insert(my_id, tx);

        while let Some(result) = user_ws_rx.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("websocket error(uid={my_id}): {e}");
                    break;
                }
            };
            user_message(my_id, msg).await;
        }

        user_disconnected(my_id).await;
    };
    tokio::task::spawn(fut);
}
async fn user_message(my_id: usize, msg: Message) {
    let msg = if let Ok(s) = msg.as_str() {
        s
    } else {
        return;
    };

    let new_msg = format!("<User#{my_id}>: {msg}");

    // New message from this user, send it to everyone else (except same uid)...
    for (&uid, tx) in ONLINE_USERS.read().await.iter() {
        if my_id != uid {
            if let Err(_disconnected) = tx.send(Ok(Message::text(new_msg.clone()))) {
                // The tx is disconnected, our `user_disconnected` code
                // should be happening in another task, nothing more to
                // do here.
            }
        }
    }
}

async fn user_disconnected(my_id: usize) {
    eprintln!("good bye user: {my_id}");
    // Stream closed up, so remove from the user list
    ONLINE_USERS.write().await.remove(&my_id);
}

#[derive(RustEmbed)]
#[folder = "static"]
struct Assets;

// Handler for English greeting
#[handler]
async fn hello() -> &'static str {
    "Hello World"
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct User {
    id: usize,
    name: String,
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
        .push(Router::with_path("ws").goal(user_connected))
        .push(static_files);

    // Print router structure for debugging
    println!("{:?}", router);

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
