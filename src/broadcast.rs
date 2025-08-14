use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use salvo::prelude::*;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};

use futures_util::{FutureExt, StreamExt};
use salvo::http::Mime;
use salvo::http::headers::ContentType;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, interval};
use tokio_stream::wrappers::UnboundedReceiverStream;

// 为每个连接生成唯一ID
static CONNECTION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct Connection {
    pub connection_id: u64,
    pub sender: mpsc::UnboundedSender<Result<Message, salvo::Error>>,
}

type BroadcastUsers = RwLock<HashMap<String, Vec<Connection>>>;

pub static BROADCAST_USERS: LazyLock<BroadcastUsers> = LazyLock::new(BroadcastUsers::default);

#[handler]
pub async fn broadcast_subscribe(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
    let string_uid = req
        .query::<String>("id")
        .ok_or_else(|| StatusError::bad_request().detail("Missing 'id' query parameter"))?;
    if string_uid.is_empty() {
        return Err(StatusError::bad_request().detail("'id' query parameter cannot be empty"));
    }
    WebSocketUpgrade::new()
        .upgrade(req, res, |ws| handle_broadcast_socket(ws, string_uid))
        .await
}

async fn handle_broadcast_socket(ws: WebSocket, my_id: String) {
    let connection_id = CONNECTION_COUNTER.fetch_add(1, Ordering::SeqCst);
    tracing::info!(
        "new broadcast user: {} (connection_id: {})",
        my_id,
        connection_id
    );

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

    // 心跳任务
    let ping_task = async move {
        let mut ping_interval = interval(Duration::from_secs(30));
        ping_interval.tick().await; // 跳过第一次触发

        loop {
            ping_interval.tick().await;
            if tx_clone_for_ping.send(Ok(Message::ping(vec![]))).is_err() {
                tracing::debug!(
                    "Failed to send ping to broadcast subscriber {my_id_clone_for_ping}, connection likely closed"
                );
                break;
            }
            tracing::debug!("Sent ping to broadcast subscriber: {my_id_clone_for_ping}");
        }
    };
    tokio::task::spawn(ping_task);

    let fut = async move {
        // 将连接添加到广播用户池
        {
            let mut users_map = BROADCAST_USERS.write().await;
            let connection = Connection {
                connection_id,
                sender: tx,
            };
            users_map
                .entry(my_id_clone_for_task.clone())
                .or_default()
                .push(connection);
        }

        // 处理接收到的消息（忽略所有消息，只处理 pong）
        while let Some(result) = user_ws_rx.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_pong() {
                        tracing::debug!(
                            "Received pong from broadcast subscriber: {} (connection_id: {}), ignoring",
                            my_id_clone_for_task,
                            connection_id
                        );
                        continue;
                    }
                    // 忽略其他所有消息
                    tracing::debug!(
                        "Ignoring message from broadcast subscriber: {} (connection_id: {})",
                        my_id_clone_for_task,
                        connection_id
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "WebSocket error for broadcast subscriber {} (connection_id: {}): {:?}",
                        my_id_clone_for_task,
                        connection_id,
                        e
                    );
                    break;
                }
            };
        }

        broadcast_user_disconnected(my_id_clone_for_task, connection_id).await;
    };
    tokio::task::spawn(fut);
}

async fn broadcast_user_disconnected(my_id: String, connection_id: u64) {
    tracing::info!(
        "broadcast subscriber disconnected: {} (connection_id: {})",
        my_id,
        connection_id
    );

    let mut users_map = BROADCAST_USERS.write().await;
    if let Some(connections) = users_map.get_mut(&my_id) {
        connections.retain(|conn| conn.connection_id != connection_id);

        // 如果没有连接了，移除整个条目
        if connections.is_empty() {
            users_map.remove(&my_id);
        }
    }
}

#[handler]
pub async fn broadcast_publish(req: &mut Request, res: &mut Response) {
    let string_uid = req.query::<String>("id").unwrap_or_default();
    if string_uid.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.body("Missing 'id' query parameter for /broad/pub");
        return;
    }

    let content_type = req
        .content_type()
        .unwrap_or_else(|| Mime::from(ContentType::octet_stream()));
    let body_bytes = match req.payload().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read payload for /broad/pub: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.body("Failed to read request body");
            return;
        }
    };

    // 构造消息
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

    // 发送给所有订阅此 id 的连接
    let users_map = BROADCAST_USERS.read().await;
    if let Some(connections) = users_map.get(&string_uid) {
        let mut failed_connection_ids = Vec::new();

        for connection in connections.iter() {
            if connection.sender.send(Ok(msg.clone())).is_err() {
                failed_connection_ids.push(connection.connection_id);
                tracing::warn!(
                    "Failed to send broadcast message to user {} (connection_id: {}), connection will be removed",
                    string_uid,
                    connection.connection_id
                );
            }
        }

        // 清理失败的连接
        if !failed_connection_ids.is_empty() {
            drop(users_map);
            let mut users_map = BROADCAST_USERS.write().await;
            if let Some(connections) = users_map.get_mut(&string_uid) {
                connections.retain(|conn| !failed_connection_ids.contains(&conn.connection_id));

                // 如果没有连接了，移除整个条目
                if connections.is_empty() {
                    users_map.remove(&string_uid);
                }
            }
        }
    }

    res.status_code(StatusCode::OK);
}
