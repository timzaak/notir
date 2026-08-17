use std::sync::LazyLock;
use std::time::Duration;

use salvo::http::body::{BodySender, ResBody};
use salvo::prelude::*;
use salvo::websocket::Message;

use bytes::Bytes;
use dashmap::DashMap;
use nanoid::nanoid;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::broadcast::BROADCAST_USERS;

/// 单个连接同时只允许一个在途传输，分块通道容量即背压窗口
const TRANSFER_CHANNEL_CAPACITY: usize = 16;
/// 相邻分块之间的最大间隔，超时视为传输死亡
const CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OFFERS_PER_CONNECTION: usize = 32;
/// 文件名截断上限（字节），避免畸形头部长度
const MAX_FILE_NAME_BYTES: usize = 255;

/// 客户端通过广播 WS 发来的文件操作（text JSON 帧）
#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientOp {
    Offer {
        #[serde(default, rename = "offerId")]
        offer_id: Option<String>,
        name: String,
        size: u64,
        #[serde(default)]
        mime: Option<String>,
    },
    Done,
    Abort,
}

#[derive(Debug, Clone)]
pub struct Offer {
    pub room_id: String,
    pub conn_id: u64,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

#[derive(Debug)]
pub(crate) enum TransferEvent {
    Chunk(Bytes),
    Done,
    Aborted,
}

pub static FILE_OFFERS: LazyLock<DashMap<String, Offer>> = LazyLock::new(DashMap::default);
pub(crate) static ACTIVE_TRANSFERS: LazyLock<DashMap<u64, mpsc::Sender<TransferEvent>>> =
    LazyLock::new(DashMap::default);

/// 原子占用传输槽；同连接已有在途传输时返回 None
pub(crate) fn try_start_transfer(conn_id: u64) -> Option<mpsc::Receiver<TransferEvent>> {
    let (tx, rx) = mpsc::channel(TRANSFER_CHANNEL_CAPACITY);
    if let dashmap::Entry::Vacant(vacant) = ACTIVE_TRANSFERS.entry(conn_id) {
        vacant.insert(tx);
        Some(rx)
    } else {
        None
    }
}

/// 处理广播 WS 上的 text 文件操作指令；非文件操作的消息静默忽略
pub async fn handle_client_op(room_id: &str, conn_id: u64, text: &str) {
    let op: ClientOp = match serde_json::from_str(text) {
        Ok(op) => op,
        Err(e) => {
            tracing::debug!(
                "ignoring non-file-op text message from broadcast subscriber {room_id} (conn {conn_id}): {e}"
            );
            return;
        }
    };

    match op {
        ClientOp::Offer {
            offer_id,
            name,
            size,
            mime,
        } => {
            let offered = FILE_OFFERS
                .iter()
                .filter(|entry| entry.value().conn_id == conn_id)
                .count();
            if offered >= MAX_OFFERS_PER_CONNECTION {
                tracing::warn!(
                    "rejecting file offer from {room_id} (conn {conn_id}): too many offers"
                );
                send_control(
                    room_id,
                    conn_id,
                    &json!({"op": "error", "message": "too many offers"}),
                )
                .await;
                return;
            }

            let mut name = name.trim().to_string();
            if name.is_empty() {
                name = "unnamed".to_string();
            }
            while name.len() > MAX_FILE_NAME_BYTES {
                name.pop();
            }

            let file_id = nanoid!();
            FILE_OFFERS.insert(
                file_id.clone(),
                Offer {
                    room_id: room_id.to_string(),
                    conn_id,
                    name: name.clone(),
                    size,
                    mime: mime
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                },
            );
            tracing::info!(
                "file offer registered: room={room_id} conn={conn_id} file={file_id} name={name} size={size}"
            );

            let mut control = json!({"op": "offer_ok", "fileId": file_id});
            if let Some(offer_id) = offer_id {
                control["offerId"] = json!(offer_id);
            }
            send_control(room_id, conn_id, &control).await;
        }
        ClientOp::Done => finish_transfer(conn_id, TransferEvent::Done).await,
        ClientOp::Abort => finish_transfer(conn_id, TransferEvent::Aborted).await,
    }
}
/// 将持有方发来的二进制分块路由到在途传输；通道满时阻塞形成背压
pub async fn route_chunk(conn_id: u64, bytes: Bytes) {
    let tx = match ACTIVE_TRANSFERS.get(&conn_id) {
        Some(tx) => tx.clone(),
        None => return,
    };
    let _ = tx.send(TransferEvent::Chunk(bytes)).await;
}

/// 持有方连接断开：清掉它的全部 offer 并中止在途传输
pub async fn holder_disconnected(room_id: &str, conn_id: u64) {
    FILE_OFFERS.retain(|_, offer| offer.conn_id != conn_id);
    if let Some((_, tx)) = ACTIVE_TRANSFERS.remove(&conn_id) {
        let _ = tx.send(TransferEvent::Aborted).await;
    }
    tracing::debug!("file holder gone: room={room_id} conn={conn_id}");
}

pub(crate) async fn finish_transfer(conn_id: u64, event: TransferEvent) {
    if let Some((_, tx)) = ACTIVE_TRANSFERS.remove(&conn_id) {
        let _ = tx.send(event).await;
    }
}

/// 向房间内指定广播连接发送控制消息（binary JSON 帧）
async fn send_control(room_id: &str, conn_id: u64, value: &serde_json::Value) -> bool {
    let payload = value.to_string();
    let users_map = BROADCAST_USERS.read().await;
    let Some(connections) = users_map.get(room_id) else {
        return false;
    };
    let Some(connection) = connections.iter().find(|c| c.connection_id == conn_id) else {
        return false;
    };
    connection
        .sender
        .send(Ok(Message::binary(payload.into_bytes())))
        .is_ok()
}

async fn holder_alive(offer: &Offer) -> bool {
    let users_map = BROADCAST_USERS.read().await;
    users_map
        .get(&offer.room_id)
        .is_some_and(|connections| connections.iter().any(|c| c.connection_id == offer.conn_id))
}

#[handler]
pub async fn status(req: &mut Request, res: &mut Response) {
    let file_id = req.param::<String>("file_id").unwrap_or_default();
    let available = match FILE_OFFERS.get(&file_id) {
        Some(entry) => {
            let offer = entry.value().clone();
            drop(entry);
            let alive = holder_alive(&offer).await;
            if !alive {
                FILE_OFFERS.remove(&file_id);
            }
            alive
        }
        None => false,
    };
    res.render(Json(json!({ "available": available })));
}

#[handler]
pub async fn download(req: &mut Request, res: &mut Response) {
    let file_id = req.param::<String>("file_id").unwrap_or_default();
    if file_id.is_empty() {
        res.status_code(StatusCode::BAD_REQUEST);
        res.render(Json(json!({"error": "missing file_id"})));
        return;
    }

    let Some(offer) = FILE_OFFERS.get(&file_id).map(|e| e.value().clone()) else {
        res.status_code(StatusCode::NOT_FOUND);
        res.render(Json(json!({"error": "unknown file_id"})));
        return;
    };

    if !holder_alive(&offer).await {
        FILE_OFFERS.remove(&file_id);
        res.status_code(StatusCode::GONE);
        res.render(Json(json!({"error": "file holder is offline"})));
        return;
    }

    let Some(rx) = try_start_transfer(offer.conn_id) else {
        res.status_code(StatusCode::CONFLICT);
        res.render(Json(
            json!({"error": "another transfer is in progress on this holder, retry shortly"}),
        ));
        return;
    };

    if !send_control(
        &offer.room_id,
        offer.conn_id,
        &json!({"op": "pull", "fileId": file_id}),
    )
    .await
    {
        ACTIVE_TRANSFERS.remove(&offer.conn_id);
        res.status_code(StatusCode::GONE);
        res.render(Json(json!({"error": "file holder is offline"})));
        return;
    }

    let headers = res.headers_mut();
    headers.insert(
        salvo::http::header::CONTENT_TYPE,
        offer
            .mime
            .parse()
            .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
    );
    headers.insert(
        salvo::http::header::CONTENT_LENGTH,
        salvo::http::HeaderValue::from(offer.size),
    );
    headers.insert(
        salvo::http::header::CONTENT_DISPOSITION,
        format!(
            "attachment; filename*=UTF-8''{}",
            percent_encode(&offer.name)
        )
        .parse()
        .unwrap(),
    );

    let (body_tx, body) = ResBody::channel();
    res.status_code(StatusCode::OK);
    res.body(body);

    tokio::spawn(forward_transfer(
        offer.room_id.clone(),
        offer.conn_id,
        offer.name.clone(),
        rx,
        body_tx,
    ));
}

/// 把传输事件泵进下载响应；出口处清理传输槽，下载方中断时通知持有方停止
async fn forward_transfer(
    room_id: String,
    conn_id: u64,
    name: String,
    mut rx: mpsc::Receiver<TransferEvent>,
    mut body_tx: BodySender,
) {
    let mut receiver_gone = false;
    loop {
        match timeout(CHUNK_IDLE_TIMEOUT, rx.recv()).await {
            Ok(Some(TransferEvent::Chunk(bytes))) => {
                if body_tx.send_data(bytes).await.is_err() {
                    receiver_gone = true;
                    break;
                }
            }
            Ok(Some(TransferEvent::Done)) => break,
            Ok(Some(TransferEvent::Aborted)) => {
                tracing::info!(
                    "file transfer aborted by holder: room={room_id} conn={conn_id} name={name}"
                );
                body_tx.send_error(std::io::Error::other("transfer aborted by holder"));
                break;
            }
            Ok(None) => {
                tracing::warn!(
                    "file transfer ended without done: room={room_id} conn={conn_id} name={name}"
                );
                body_tx.send_error(std::io::Error::other("transfer ended unexpectedly"));
                break;
            }
            Err(_) => {
                tracing::warn!(
                    "file transfer idle timeout: room={room_id} conn={conn_id} name={name}"
                );
                body_tx.send_error(std::io::Error::other("transfer idle timeout"));
                break;
            }
        }
    }
    ACTIVE_TRANSFERS.remove(&conn_id);
    if receiver_gone {
        tracing::info!("download cancelled by receiver: room={room_id} conn={conn_id} name={name}");
        send_control(&room_id, conn_id, &json!({"op": "cancel"})).await;
    }
}

/// RFC 5987 percent-encoding：非保留字符原样，其余编码为 %XX
pub(crate) fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
