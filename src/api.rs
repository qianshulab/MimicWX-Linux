//! HTTP API 服务
//!
//! 提供 REST + WebSocket 接口:
//! - GET  /status        — 服务状态 (免认证)
//! - GET  /contacts      — 联系人列表 (数据库)
//! - GET  /sessions      — 会话列表 (优先数据库)
//! - GET  /messages/new  — 增量新消息 (数据库)
//! - POST /send          — 发送消息 (AT-SPI)
//! - POST /chat          — 切换聊天 (AT-SPI)
//! - POST /listen        — 添加监听 (弹出独立窗口)
//! - DELETE /listen      — 移除监听
//! - GET  /listen        — 监听列表
//! - GET  /debug/tree    — AT-SPI2 控件树
//! - GET  /ws            — WebSocket 实时推送

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{
        header::{
            ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, ETAG, RANGE,
        },
        HeaderMap, HeaderValue, Request, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::broadcast;
use tokio_util::io::ReaderStream;
use tracing::{debug, info, warn};

use crate::atspi::AtSpi;
use crate::db::{DbManager, DbMessage};
use crate::input::InputEngine;
use crate::wechat::WeChat;

// =====================================================================
// 共享状态
// =====================================================================

/// API 独立消息游标缓存。
/// 数据库监听只写入一次，多个调用方可以用各自的 `after` 游标读取，互不抢消息。
pub struct MessageStore {
    capacity: usize,
    next_cursor: AtomicU64,
    legacy_cursor: AtomicU64,
    messages: tokio::sync::RwLock<VecDeque<BufferedMessage>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BufferedMessage {
    pub cursor: u64,
    #[serde(flatten)]
    pub message: DbMessage,
}

impl MessageStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(128),
            next_cursor: AtomicU64::new(0),
            legacy_cursor: AtomicU64::new(0),
            messages: tokio::sync::RwLock::new(VecDeque::new()),
        }
    }

    pub async fn push(&self, message: DbMessage) -> BufferedMessage {
        let cursor = self.next_cursor.fetch_add(1, Ordering::Relaxed) + 1;
        let stored = BufferedMessage { cursor, message };
        let mut messages = self.messages.write().await;
        messages.push_back(stored.clone());
        while messages.len() > self.capacity {
            messages.pop_front();
        }
        stored
    }

    pub fn latest_cursor(&self) -> u64 {
        self.next_cursor.load(Ordering::Relaxed)
    }

    pub async fn query(
        &self,
        after: u64,
        limit: usize,
        chat: Option<&str>,
        sender: Option<&str>,
        direction: Option<&str>,
    ) -> (Vec<BufferedMessage>, bool) {
        let limit = limit.clamp(1, 500);
        let messages = self.messages.read().await;
        let mut matched: Vec<BufferedMessage> = messages
            .iter()
            .filter(|entry| entry.cursor > after)
            .filter(|entry| {
                chat.is_none_or(|wanted| {
                    entry.message.conversation_id == wanted
                        || entry.message.conversation_name == wanted
                })
            })
            .filter(|entry| {
                sender.is_none_or(|wanted| {
                    entry.message.sender_id == wanted || entry.message.sender_name == wanted
                })
            })
            .filter(|entry| direction.is_none_or(|wanted| entry.message.direction == wanted))
            .take(limit + 1)
            .cloned()
            .collect();
        let has_more = matched.len() > limit;
        matched.truncate(limit);
        (matched, has_more)
    }

    pub async fn find_message(&self, message_id: &str) -> Option<BufferedMessage> {
        self.messages
            .read()
            .await
            .iter()
            .rev()
            .find(|entry| entry.message.message_id == message_id)
            .cloned()
    }

    async fn take_legacy(&self, limit: usize) -> Vec<BufferedMessage> {
        let after = self.legacy_cursor.load(Ordering::Relaxed);
        let (messages, _) = self.query(after, limit, None, None, None).await;
        if let Some(last) = messages.last() {
            self.legacy_cursor.store(last.cursor, Ordering::Relaxed);
        }
        messages
    }
}

pub struct AppState {
    pub wechat: Arc<WeChat>,
    pub atspi: Arc<AtSpi>,
    /// InputEngine 命令队列 (替代 Mutex, 消除长持锁)
    pub input_tx: tokio::sync::mpsc::Sender<InputCommand>,
    pub tx: broadcast::Sender<String>,
    /// 数据库管理器 (密钥获取成功时可用)
    pub db: Option<Arc<DbManager>>,
    /// 多客户端安全的实时消息缓存。
    pub messages: Arc<MessageStore>,
    /// API 认证 Token (None = 不启用认证)
    pub api_token: Option<String>,
    /// 启动时间 (用于 uptime 计算)
    pub start_time: std::time::Instant,
    /// 配置文件路径 (用于 /reload 和 /listen 持久化)
    pub config_path: Option<std::path::PathBuf>,
}

// =====================================================================
// InputEngine Actor
// =====================================================================

use tokio::sync::oneshot;

/// InputEngine 命令 (经 mpsc 队列发送给 actor)
pub enum InputCommand {
    SendMessage {
        to: String,
        text: String,
        at: Vec<String>,
        skip_verify: bool,
        reply: oneshot::Sender<anyhow::Result<(bool, bool, String)>>,
    },
    SendImage {
        to: String,
        image_path: String,
        reply: oneshot::Sender<anyhow::Result<(bool, bool, String)>>,
    },
    SendFile {
        to: String,
        file_path: String,
        reply: oneshot::Sender<anyhow::Result<(bool, bool, String)>>,
    },
    ChatWith {
        who: String,
        reply: oneshot::Sender<anyhow::Result<Option<String>>>,
    },
    AddListen {
        who: String,
        reply: oneshot::Sender<anyhow::Result<bool>>,
    },
    RemoveListen {
        who: String,
        reply: oneshot::Sender<bool>,
    },
}

/// 启动 InputEngine actor (在独立 task 中顺序执行命令)
pub fn spawn_input_actor(
    mut engine: InputEngine,
    wechat: Arc<WeChat>,
    mut rx: tokio::sync::mpsc::Receiver<InputCommand>,
) {
    tokio::spawn(async move {
        info!("[input] InputEngine actor 已启动");
        while let Some(cmd) = rx.recv().await {
            match cmd {
                InputCommand::SendMessage {
                    to,
                    text,
                    at,
                    skip_verify,
                    reply,
                } => {
                    // 自动恢复: 独立窗口失效时尝试重建
                    if !wechat.check_listen_window(&to).await {
                        wechat.try_recover_listen_window(&mut engine, &to).await;
                    }
                    let result = wechat
                        .send_message(&mut engine, &to, &text, &at, skip_verify)
                        .await;
                    let _ = reply.send(result);
                }
                InputCommand::SendImage {
                    to,
                    image_path,
                    reply,
                } => {
                    // 自动恢复: 独立窗口失效时尝试重建
                    if !wechat.check_listen_window(&to).await {
                        wechat.try_recover_listen_window(&mut engine, &to).await;
                    }
                    let result = wechat.send_image(&mut engine, &to, &image_path).await;
                    let _ = reply.send(result);
                }
                InputCommand::SendFile {
                    to,
                    file_path,
                    reply,
                } => {
                    // Generic files always use the main window.  The independent
                    // chat window has a different toolbar layout across WeChat
                    // releases, while the main-window chooser is stable.
                    let result = wechat.send_file(&mut engine, &to, &file_path).await;
                    let _ = reply.send(result);
                }
                InputCommand::ChatWith { who, reply } => {
                    let result = wechat.chat_with(&mut engine, &who).await;
                    let _ = reply.send(result);
                }
                InputCommand::AddListen { who, reply } => {
                    let result = wechat.add_listen(&mut engine, &who).await;
                    let _ = reply.send(result);
                }
                InputCommand::RemoveListen { who, reply } => {
                    let result = wechat.remove_listen(&engine, &who).await;
                    let _ = reply.send(result);
                }
            }
        }
        info!("[input] InputEngine actor 已停止");
    });
}

// =====================================================================
// 工具函数
// =====================================================================

/// 简单的 URL percent decode (%XX → 字节)
fn percent_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.as_bytes().iter();
    while let Some(&b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().copied().unwrap_or(0);
            let lo = chars.next().copied().unwrap_or(0);
            if let (Some(h), Some(l)) = (hex_val(hi), hex_val(lo)) {
                bytes.push(h << 4 | l);
                continue;
            }
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 轻量伪随机 u16 (无需引入 rand crate, 用时间纳秒低位)
fn rand_u16() -> u16 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (t.subsec_nanos() ^ (t.as_millis() as u32)) as u16
}

// =====================================================================
// 统一错误响应
// =====================================================================

/// API 错误类型 (带 HTTP 状态码)
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: msg.into(),
        }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn range_not_satisfiable(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::RANGE_NOT_SATISFIABLE,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

// =====================================================================
// 认证中间件
// =====================================================================

/// Token 认证中间件
/// 检查 Header `Authorization: Bearer <token>` 或 Query `?token=<token>`
async fn auth_layer(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    let token = match &state.api_token {
        Some(t) => t,
        None => return Ok(next.run(req).await), // 未配置 token, 跳过认证
    };

    // 1. 检查 Authorization header
    if let Some(auth) = req.headers().get("authorization") {
        if let Ok(auth_str) = auth.to_str() {
            if let Some(bearer) = auth_str.strip_prefix("Bearer ") {
                if bearer.trim() == token {
                    return Ok(next.run(req).await);
                }
            }
        }
    }

    // 2. 检查 query param ?token=xxx (需 URL decode)
    if let Some(query) = req.uri().query() {
        for pair in query.split('&') {
            if let Some(val) = pair.strip_prefix("token=") {
                // URL decode: %23 → #, %20 → space, etc.
                let decoded = percent_decode(val);
                if decoded == *token {
                    return Ok(next.run(req).await);
                }
            }
        }
    }

    warn!("🔒 API 认证失败: {}", req.uri().path());
    Err(StatusCode::UNAUTHORIZED)
}

// =====================================================================
// 路由
// =====================================================================

pub fn build_router(state: Arc<AppState>) -> Router {
    // 需要认证的路由
    let protected = Router::new()
        .route("/contacts", get(get_contacts))
        .route("/messages", get(get_messages))
        .route("/messages/history", get(get_message_history))
        .route("/messages/new", get(get_new_messages))
        .route("/messages/send", post(send_message_v2))
        .route("/messages/reply", post(reply_message))
        .route("/attachments/{id}", get(download_attachment))
        .route("/send", post(send_message))
        .route("/send_image", post(send_image))
        .route("/send_file", post(send_file))
        .route("/messages/send-file", post(send_file))
        .route("/sessions", get(get_sessions))
        .route("/chat", post(chat_with))
        .route("/listen", get(get_listen_list))
        .route("/listen", post(add_listen))
        .route("/listen", delete(remove_listen))
        .route("/command", post(exec_command))
        .route("/debug/tree", get(get_tree))
        .route("/debug/sessions", get(get_session_tree))
        .route("/ws", get(ws_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_layer));

    // 免认证路由
    Router::new()
        .route("/status", get(get_status))
        .merge(protected)
        .layer(tower_http::cors::CorsLayer::permissive()) // ⑩ CORS 支持
        .with_state(state)
}

// =====================================================================
// 请求/响应类型
// =====================================================================

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    version: String,
    listen_count: usize,
    db_available: bool,
    contacts: usize,
    uptime_secs: u64,
    message_cursor: u64,
}

#[derive(Debug, Deserialize)]
struct MessageQuery {
    /// 每个调用方持有自己的 cursor，下次传回 after，不会与其他调用方互相消费。
    #[serde(default)]
    after: u64,
    #[serde(default = "default_message_limit")]
    limit: usize,
    chat: Option<String>,
    sender: Option<String>,
    direction: Option<String>,
}

fn default_message_limit() -> usize {
    100
}

#[derive(Serialize)]
struct MessageListResponse {
    messages: Vec<BufferedMessage>,
    next_cursor: u64,
    latest_cursor: u64,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct MessageHistoryQuery {
    #[serde(default)]
    since: i64,
    /// Stable offset within the ascending result set for this exact `since` value.
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_message_limit")]
    limit: usize,
    chat: Option<String>,
    sender: Option<String>,
    direction: Option<String>,
}

#[derive(Serialize)]
struct MessageHistoryResponse {
    messages: Vec<DbMessage>,
    next_offset: usize,
    checkpoint_time: i64,
    /// Compatibility alias for checkpoint_time.  Pagination uses next_offset.
    next_since: i64,
    has_more: bool,
}

#[derive(Deserialize)]
struct SendRequest {
    to: String,
    text: String,
    /// 要 @ 的人的显示名列表 (可选)
    #[serde(default)]
    at: Vec<String>,
}

#[derive(Deserialize)]
struct SendImageRequest {
    to: String,
    /// base64 编码的图片数据
    file: String,
    /// 文件名 (可选, 用于推断 MIME 类型)
    #[serde(default = "default_image_name")]
    name: String,
}

fn default_image_name() -> String {
    "image.png".to_string()
}

#[derive(Serialize)]
struct SendResponse {
    sent: bool,
    verified: bool,
    message: String,
}

#[derive(Deserialize)]
struct ChatRequest {
    who: String,
}

#[derive(Serialize)]
struct ChatResponse {
    success: bool,
    chat_name: Option<String>,
}

#[derive(Deserialize)]
struct ListenRequest {
    who: String,
}

#[derive(Serialize)]
struct ListenResponse {
    success: bool,
    message: String,
}

// =====================================================================
// Handlers
// =====================================================================

async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let status = state.wechat.check_status().await;
    let listen_count = state.wechat.get_listen_list().await.len();
    let db_available = state.db.is_some();
    let contacts = if let Some(ref d) = state.db {
        d.get_contacts().await.len()
    } else {
        0
    };
    let uptime_secs = state.start_time.elapsed().as_secs();
    Json(StatusResponse {
        status: status.to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
        listen_count,
        db_available,
        contacts,
        uptime_secs,
        message_cursor: state.messages.latest_cursor(),
    })
}

#[derive(Deserialize)]
struct SendFileRequest {
    #[serde(alias = "conversation")]
    to: String,
    /// base64 编码文件，最大 100 MiB（解码后）。
    file: String,
    name: String,
}

#[derive(Deserialize)]
struct SendMessageV2Request {
    /// conversation_id 或微信显示名/备注名。
    conversation: String,
    text: String,
    #[serde(default)]
    at: Vec<String>,
}

#[derive(Deserialize)]
struct ReplyMessageRequest {
    message_id: String,
    text: String,
    #[serde(default = "default_true")]
    mention_sender: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
struct MessageSendResponse {
    sent: bool,
    verified: bool,
    message: String,
    conversation_id: String,
    conversation_name: String,
    reply_to_message_id: Option<String>,
}

/// 联系人列表 (从数据库)
async fn get_contacts(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| ApiError::unavailable("数据库不可用"))?;
    let contacts = db.get_contacts().await;
    Ok(Json(serde_json::json!({ "contacts": contacts })))
}

/// 多客户端消息接口。每个客户端保存自己的 next_cursor，并作为下次 after 传入。
async fn get_messages(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessageQuery>,
) -> Json<MessageListResponse> {
    let (messages, has_more) = state
        .messages
        .query(
            query.after,
            query.limit,
            query.chat.as_deref(),
            query.sender.as_deref(),
            query.direction.as_deref(),
        )
        .await;
    let next_cursor = messages
        .last()
        .map(|item| item.cursor)
        .unwrap_or(query.after);
    Json(MessageListResponse {
        messages,
        next_cursor,
        latest_cursor: state.messages.latest_cursor(),
        has_more,
    })
}

/// 从加密微信数据库补拉历史消息。固定 since 并推进 offset，按 message_id 去重。
async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessageHistoryQuery>,
) -> Result<Json<MessageHistoryResponse>, ApiError> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| ApiError::unavailable("数据库不可用"))?;
    let requested_limit = query.limit.clamp(1, 500);
    if query.offset > 100_000 {
        return Err(ApiError::bad_request("历史分页 offset 不能超过 100000"));
    }
    let (mut messages, has_more) = db
        .get_message_history(
            query.chat.as_deref(),
            query.since,
            query.offset,
            requested_limit,
        )
        .await
        .map_err(|error| ApiError::internal(format!("历史消息查询失败: {error}")))?;
    // Advance over the scanned base page even when optional filters remove all
    // returned messages; otherwise a narrow filter could loop forever.
    let scanned_count = messages.len();
    let checkpoint_time = messages
        .last()
        .map(|message| message.create_time)
        .unwrap_or(query.since);
    messages.retain(|message| {
        query
            .sender
            .as_ref()
            .is_none_or(|wanted| &message.sender_id == wanted || &message.sender_name == wanted)
            && query
                .direction
                .as_ref()
                .is_none_or(|wanted| &message.direction == wanted)
    });
    Ok(Json(MessageHistoryResponse {
        messages,
        next_offset: query.offset.saturating_add(scanned_count),
        checkpoint_time,
        next_since: checkpoint_time,
        has_more,
    }))
}

/// 旧版单消费者增量接口；新集成应使用 /messages + after 游标。
async fn get_new_messages(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessageQuery>,
) -> Json<Vec<BufferedMessage>> {
    if query.after > 0
        || query.chat.is_some()
        || query.sender.is_some()
        || query.direction.is_some()
    {
        let (messages, _) = state
            .messages
            .query(
                query.after,
                query.limit,
                query.chat.as_deref(),
                query.sender.as_deref(),
                query.direction.as_deref(),
            )
            .await;
        Json(messages)
    } else {
        Json(state.messages.take_legacy(query.limit).await)
    }
}

fn parse_range_header(
    value: Option<&HeaderValue>,
    total: u64,
) -> Result<Option<(u64, u64)>, ApiError> {
    let Some(value) = value else { return Ok(None) };
    let raw = value
        .to_str()
        .map_err(|_| ApiError::range_not_satisfiable("Range 格式无效"))?;
    let range = raw
        .strip_prefix("bytes=")
        .ok_or_else(|| ApiError::range_not_satisfiable("仅支持 bytes Range"))?;
    if range.contains(',') || total == 0 {
        return Err(ApiError::range_not_satisfiable("仅支持单段 Range"));
    }
    let (start_raw, end_raw) = range
        .split_once('-')
        .ok_or_else(|| ApiError::range_not_satisfiable("Range 格式无效"))?;
    let (start, end) = if start_raw.is_empty() {
        let suffix = end_raw
            .parse::<u64>()
            .map_err(|_| ApiError::range_not_satisfiable("Range 格式无效"))?;
        if suffix == 0 {
            return Err(ApiError::range_not_satisfiable("Range 长度必须大于 0"));
        }
        (total.saturating_sub(suffix.min(total)), total - 1)
    } else {
        let start = start_raw
            .parse::<u64>()
            .map_err(|_| ApiError::range_not_satisfiable("Range 起点无效"))?;
        let end = if end_raw.is_empty() {
            total - 1
        } else {
            end_raw
                .parse::<u64>()
                .map_err(|_| ApiError::range_not_satisfiable("Range 终点无效"))?
                .min(total - 1)
        };
        (start, end)
    };
    if start >= total || start > end {
        return Err(ApiError::range_not_satisfiable("Range 超出文件范围"));
    }
    Ok(Some((start, end)))
}

/// 流式下载微信已落盘的附件。定位符只能在当前账号的附件白名单目录中解析。
async fn download_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| ApiError::unavailable("数据库不可用"))?;
    let resolved = db
        .resolve_attachment(&id)
        .await
        .map_err(|_| ApiError::not_found("附件尚未下载到微信本地目录"))?;
    let mut file = tokio::fs::File::open(&resolved.path)
        .await
        .map_err(|e| ApiError::internal(format!("打开附件失败: {e}")))?;
    let total = resolved.size;
    let requested = parse_range_header(headers.get(RANGE), total)?;
    let (status, start, end) = match requested {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end),
        None if total > 0 => (StatusCode::OK, 0, total - 1),
        None => (StatusCode::OK, 0, 0),
    };
    let length = if total == 0 { 0 } else { end - start + 1 };
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| ApiError::internal(format!("定位附件失败: {e}")))?;
    }
    let stream = ReaderStream::new(file.take(length));
    let encoded_name = utf8_percent_encode(&resolved.name, NON_ALPHANUMERIC).to_string();
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"attachment.bin\"; filename*=UTF-8''{encoded_name}"
    ))
    .map_err(|_| ApiError::internal("附件文件名响应头无效"))?;
    let etag = HeaderValue::from_str(&format!("\"{}\"", resolved.md5.as_deref().unwrap_or(&id)))
        .map_err(|_| ApiError::internal("附件 ETag 无效"))?;

    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/octet-stream")
        .header(CONTENT_DISPOSITION, disposition)
        .header(CONTENT_LENGTH, length.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header(ETAG, etag);
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| ApiError::internal(format!("创建附件响应失败: {e}")))
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    dispatch_text(&state, req.to, req.text, req.at)
        .await
        .map(Json)
}

async fn dispatch_text(
    state: &Arc<AppState>,
    to: String,
    text: String,
    at: Vec<String>,
) -> Result<SendResponse, ApiError> {
    if to.trim().is_empty() {
        return Err(ApiError::bad_request("会话不能为空"));
    }
    if text.trim().is_empty() {
        return Err(ApiError::bad_request("消息内容不能为空"));
    }
    if text.len() > 64 * 1024 {
        return Err(ApiError::bad_request("单条文本消息不能超过 64 KiB"));
    }
    if at.len() > 100 {
        return Err(ApiError::bad_request("单条消息最多 @ 100 人"));
    }
    // DB 可用时跳过 AT-SPI 验证, 由下面的 DB 验证替代
    let has_db = state.db.is_some();

    // 在发送前订阅自发消息广播 (避免竞态: 发送期间的广播不会丢失)
    let sent_rx = state.db.as_ref().map(|db| db.subscribe_sent());

    // 发送命令到 actor
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .input_tx
        .send(InputCommand::SendMessage {
            to: to.clone(),
            text: text.clone(),
            at,
            skip_verify: has_db,
            reply: reply_tx,
        })
        .await
        .map_err(|_| ApiError::unavailable("InputEngine actor 已停止"))?;

    match reply_rx.await {
        Ok(Ok((sent, atspi_verified, message))) => {
            // DB 验证 (优先): DB 可用时用已订阅的 receiver 等待匹配
            let verified = if let Some(rx) = sent_rx {
                state
                    .db
                    .as_ref()
                    .unwrap()
                    .verify_sent(&text, rx)
                    .await
                    .unwrap_or(atspi_verified)
            } else {
                atspi_verified
            };

            let msg_json = serde_json::json!({
                "type": "sent",
                "to": to,
                "text": text,
                "verified": verified,
            });
            let _ = state.tx.send(msg_json.to_string());
            Ok(SendResponse {
                sent,
                verified,
                message,
            })
        }
        Ok(Err(e)) => Err(ApiError::internal(format!("发送失败: {e}"))),
        Err(_) => Err(ApiError::internal("actor 响应通道已关闭")),
    }
}

async fn send_message_v2(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendMessageV2Request>,
) -> Result<Json<MessageSendResponse>, ApiError> {
    let conversation_id = state
        .db
        .as_ref()
        .map(|db| db.resolve_chat_identifier(&req.conversation))
        .unwrap_or_else(|| req.conversation.clone());
    let conversation_name = state
        .db
        .as_ref()
        .map(|db| db.conversation_display_name(&conversation_id))
        .unwrap_or_else(|| req.conversation.clone());
    let response = dispatch_text(&state, conversation_name.clone(), req.text, req.at).await?;
    Ok(Json(MessageSendResponse {
        sent: response.sent,
        verified: response.verified,
        message: response.message,
        conversation_id,
        conversation_name,
        reply_to_message_id: None,
    }))
}

async fn reply_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReplyMessageRequest>,
) -> Result<Json<MessageSendResponse>, ApiError> {
    let target = state
        .messages
        .find_message(&req.message_id)
        .await
        .ok_or_else(|| {
            ApiError::not_found("消息不在实时缓存中；请使用 conversation 调用 /messages/send")
        })?;
    let message = target.message;
    let conversation_name = if message.conversation_name.is_empty() {
        message.conversation_id.clone()
    } else {
        message.conversation_name.clone()
    };
    let at = if req.mention_sender
        && message.is_group
        && !message.is_self
        && !message.sender_name.is_empty()
    {
        vec![message.sender_name.clone()]
    } else {
        Vec::new()
    };
    let response = dispatch_text(&state, conversation_name.clone(), req.text, at).await?;
    Ok(Json(MessageSendResponse {
        sent: response.sent,
        verified: response.verified,
        message: response.message,
        conversation_id: message.conversation_id,
        conversation_name,
        reply_to_message_id: Some(req.message_id),
    }))
}

async fn send_image(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendImageRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    use std::io::Write;

    // 解码 base64 图片
    use base64::Engine;
    let image_data = base64::engine::general_purpose::STANDARD
        .decode(&req.file)
        .map_err(|e| ApiError::internal(format!("base64 解码失败: {e}")))?;

    // 保存到临时文件
    let ext = if req.name.contains('.') {
        req.name.rsplit('.').next().unwrap_or("png")
    } else {
        "png"
    };
    let tmp_path = format!(
        "/tmp/mimicwx_img_{}_{:04x}.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        rand_u16(),
        ext
    );
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| ApiError::internal(format!("创建临时文件失败: {e}")))?;
        f.write_all(&image_data)
            .map_err(|e| ApiError::internal(format!("写入图片失败: {e}")))?;
    }

    // 发送命令到 actor
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .input_tx
        .send(InputCommand::SendImage {
            to: req.to.clone(),
            image_path: tmp_path.clone(),
            reply: reply_tx,
        })
        .await
        .map_err(|_| ApiError::unavailable("InputEngine actor 已停止"))?;

    let result = reply_rx.await;

    // 清理临时文件
    let _ = std::fs::remove_file(&tmp_path);

    match result {
        Ok(Ok((sent, verified, message))) => Ok(Json(SendResponse {
            sent,
            verified,
            message,
        })),
        Ok(Err(e)) => Err(ApiError::internal(format!("发送图片失败: {e}"))),
        Err(_) => Err(ApiError::internal("actor 响应通道已关闭")),
    }
}

fn validate_upload_name(name: &str) -> Result<&str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 255
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request("文件名无效"));
    }
    Ok(trimmed)
}

async fn send_file(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendFileRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    use base64::Engine;
    use std::io::Write;

    const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
    if req.file.len() > (MAX_FILE_BYTES * 4 / 3) + 16 {
        return Err(ApiError::bad_request("发送文件不能超过 100 MiB"));
    }
    let name = validate_upload_name(&req.name)?.to_string();
    let file_data = base64::engine::general_purpose::STANDARD
        .decode(&req.file)
        .map_err(|error| ApiError::bad_request(format!("base64 解码失败: {error}")))?;
    if file_data.len() > MAX_FILE_BYTES {
        return Err(ApiError::bad_request("发送文件不能超过 100 MiB"));
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_dir = std::path::PathBuf::from(format!(
        "/tmp/mimicwx_outgoing_{timestamp}_{:04x}",
        rand_u16()
    ));
    std::fs::create_dir(&temp_dir)
        .map_err(|error| ApiError::internal(format!("创建临时目录失败: {error}")))?;
    let temp_path = temp_dir.join(&name);
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&file_data)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(ApiError::internal(format!("写入临时文件失败: {error}")));
    }

    let conversation_id = state
        .db
        .as_ref()
        .map(|db| db.resolve_chat_identifier(&req.to))
        .unwrap_or_else(|| req.to.clone());
    let target = state
        .db
        .as_ref()
        .map(|db| db.conversation_display_name(&conversation_id))
        .unwrap_or_else(|| req.to.clone());
    let sent_rx = state.db.as_ref().map(|db| db.subscribe_sent());
    let (reply_tx, reply_rx) = oneshot::channel();
    if state
        .input_tx
        .send(InputCommand::SendFile {
            to: target,
            file_path: temp_path.to_string_lossy().to_string(),
            reply: reply_tx,
        })
        .await
        .is_err()
    {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(ApiError::unavailable("InputEngine actor 已停止"));
    }
    let actor_result = tokio::time::timeout(std::time::Duration::from_secs(120), reply_rx).await;

    // 微信可能异步读取文件；延迟清理，避免大文件上传过程中被删除。
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(900)).await;
        let _ = tokio::fs::remove_dir_all(temp_dir).await;
    });

    match actor_result {
        Ok(Ok(Ok((sent, atspi_verified, message)))) => {
            let verified = if sent {
                if let Some(receiver) = sent_rx {
                    state
                        .db
                        .as_ref()
                        .unwrap()
                        .verify_sent(&name, receiver)
                        .await
                        .unwrap_or(atspi_verified)
                } else {
                    atspi_verified
                }
            } else {
                false
            };
            Ok(Json(SendResponse {
                sent,
                verified,
                message,
            }))
        }
        Ok(Ok(Err(error))) => Err(ApiError::internal(format!("发送文件失败: {error}"))),
        Ok(Err(_)) => Err(ApiError::internal("actor 响应通道已关闭")),
        Err(_) => Err(ApiError::unavailable(
            "发送文件超时；微信可能仍在处理，请检查会话",
        )),
    }
}

async fn get_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // 优先使用数据库
    if let Some(db) = &state.db {
        match db.get_sessions().await {
            Ok(sessions) => return Json(serde_json::to_value(sessions).unwrap_or_default()),
            Err(e) => {
                tracing::warn!("数据库会话查询失败, fallback AT-SPI: {}", e);
            }
        }
    }
    // Fallback: AT-SPI
    let sessions = state.wechat.list_sessions().await;
    Json(serde_json::to_value(sessions).unwrap_or_default())
}

async fn chat_with(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .input_tx
        .send(InputCommand::ChatWith {
            who: req.who.clone(),
            reply: reply_tx,
        })
        .await
        .map_err(|_| ApiError::unavailable("InputEngine actor 已停止"))?;

    match reply_rx.await {
        Ok(Ok(Some(name))) => Ok(Json(ChatResponse {
            success: true,
            chat_name: Some(name),
        })),
        Ok(Ok(None)) => Ok(Json(ChatResponse {
            success: false,
            chat_name: None,
        })),
        Ok(Err(e)) => Err(ApiError::internal(format!("切换聊天失败: {e}"))),
        Err(_) => Err(ApiError::internal("actor 响应通道已关闭")),
    }
}

async fn add_listen(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListenRequest>,
) -> Result<Json<ListenResponse>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .input_tx
        .send(InputCommand::AddListen {
            who: req.who.clone(),
            reply: reply_tx,
        })
        .await
        .map_err(|_| ApiError::unavailable("InputEngine actor 已停止"))?;

    match reply_rx.await {
        Ok(Ok(true)) => Ok(Json(ListenResponse {
            success: true,
            message: format!("已添加监听: {}", req.who),
        })),
        Ok(Ok(false)) => Ok(Json(ListenResponse {
            success: false,
            message: format!("添加监听失败: {}", req.who),
        })),
        Ok(Err(e)) => Err(ApiError::internal(format!("添加监听错误: {e}"))),
        Err(_) => Err(ApiError::internal("actor 响应通道已关闭")),
    }
}

async fn remove_listen(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListenRequest>,
) -> Json<ListenResponse> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let sent = state
        .input_tx
        .send(InputCommand::RemoveListen {
            who: req.who.clone(),
            reply: reply_tx,
        })
        .await;

    let removed = if sent.is_ok() {
        reply_rx.await.unwrap_or(false)
    } else {
        false
    };
    Json(ListenResponse {
        success: removed,
        message: if removed {
            format!("已移除监听: {}", req.who)
        } else {
            format!("未找到监听: {}", req.who)
        },
    })
}

async fn get_listen_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let list = state.wechat.get_listen_list().await;
    Json(list)
}

async fn get_tree(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let max_depth = params
        .get("depth")
        .and_then(|d| d.parse::<u32>().ok())
        .unwrap_or(5)
        .min(15);
    if let Some(app) = state.wechat.find_app().await {
        let tree = state.atspi.dump_tree(&app, max_depth).await;
        Json(tree)
    } else {
        Json(vec![])
    }
}

/// 只 dump 会话容器的子树 (用于调试)
async fn get_session_tree(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(app) = state.wechat.find_app().await {
        if let Some(container) = state.wechat.find_session_list(&app).await {
            let tree = state.atspi.dump_tree(&container, 4).await;
            return Json(tree);
        }
    }
    Json(vec![])
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    debug!("🔌 WebSocket 连接建立");

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    ping_interval.tick().await; // 跳过首次

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {} // 心跳响应
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                // ⑴ WebSocket 心跳: 每 30s 发 Ping
                if socket.send(Message::Ping(vec![].into())).await.is_err() { break; }
            }
        }
    }

    debug!("🔌 WebSocket 连接断开");
}

// =====================================================================
// POST /command — 通用命令执行 (微信互通)
// =====================================================================

#[derive(Deserialize)]
struct CommandReq {
    cmd: String,
}

async fn exec_command(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommandReq>,
) -> impl IntoResponse {
    let cmd = req.cmd.trim();
    info!("[input] 收到远程命令: {cmd}");

    let result = match cmd {
        "status" => {
            let status = state.wechat.check_status().await;
            let listen_list = state.wechat.get_listen_list().await;
            let db_status = if state.db.is_some() {
                "可用"
            } else {
                "不可用"
            };
            let contacts = if let Some(ref d) = state.db {
                d.get_contacts().await.len()
            } else {
                0
            };
            let uptime = state.start_time.elapsed().as_secs();
            let h = uptime / 3600;
            let m = (uptime % 3600) / 60;
            format!(
                "📊 微信: {status}\n📊 数据库: {db_status} | 联系人: {contacts}\n📊 监听: {} 个 {:?}\n📊 运行: {h}h{m}m | v{}",
                listen_list.len(), listen_list, env!("CARGO_PKG_VERSION")
            )
        }
        "atmode" => {
            let msg = serde_json::json!({
                "type": "control",
                "cmd": "toggle_at_mode",
            });
            let _ = state.tx.send(msg.to_string());
            "📢 已发送仅@模式切换指令".to_string()
        }
        "reload" => exec_reload(&state).await,
        _ if cmd.starts_with("listen ") => {
            let who = cmd.strip_prefix("listen ").unwrap().trim();
            if who.is_empty() {
                "[err] 用法: listen <联系人/群名>".to_string()
            } else {
                exec_listen(&state, who).await
            }
        }
        _ if cmd.starts_with("unlisten ") => {
            let who = cmd.strip_prefix("unlisten ").unwrap().trim();
            if who.is_empty() {
                "[err] 用法: unlisten <联系人/群名>".to_string()
            } else {
                exec_unlisten(&state, who).await
            }
        }
        _ if cmd.starts_with("send ") => {
            let rest = cmd.strip_prefix("send ").unwrap().trim();
            if let Some((to, text)) = rest.split_once(' ') {
                exec_send(&state, to.trim(), text.trim()).await
            } else {
                "[err] 用法: send <收件人> <内容>".to_string()
            }
        }
        _ => format!("❓ 未知命令: {cmd}"),
    };

    info!("[input] 命令结果: {result}");
    Json(serde_json::json!({ "ok": true, "result": result }))
}

/// 执行 reload 命令
async fn exec_reload(state: &AppState) -> String {
    let path = match &state.config_path {
        Some(p) => p,
        None => return "[warn] 未找到配置文件路径".to_string(),
    };
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return format!("[warn] 读取配置失败: {e}"),
    };
    let new_config: crate::config::AppConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => return format!("[warn] 配置解析失败: {e}"),
    };

    let mut lines = Vec::new();

    // 更新 at_delay_ms
    let old = state.wechat.get_at_delay_ms();
    let new = new_config.timing.at_delay_ms;
    if old != new {
        state.wechat.set_at_delay_ms(new);
        lines.push(format!("[config] at_delay_ms: {old} → {new}"));
    }

    // Diff listen 列表
    let current = state.wechat.get_listen_list().await;
    let new_list = new_config.listen.auto;
    let to_add: Vec<_> = new_list
        .iter()
        .filter(|n| !current.contains(n))
        .cloned()
        .collect();
    let to_remove: Vec<_> = current
        .iter()
        .filter(|n| !new_list.contains(n))
        .cloned()
        .collect();

    for who in &to_remove {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if state
            .input_tx
            .send(InputCommand::RemoveListen {
                who: who.clone(),
                reply: reply_tx,
            })
            .await
            .is_ok()
        {
            let _ = reply_rx.await;
        }
        lines.push(format!("[listen] 移除监听: {who}"));
    }
    for who in &to_add {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if state
            .input_tx
            .send(InputCommand::AddListen {
                who: who.clone(),
                reply: reply_tx,
            })
            .await
            .is_ok()
        {
            match reply_rx.await {
                Ok(Ok(true)) => lines.push(format!("[ok] 添加监听: {who}")),
                _ => lines.push(format!("[warn] 添加失败: {who}")),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    if lines.is_empty() {
        "[config] 配置已重载 (无变化)".to_string()
    } else {
        lines.push("[config] 配置已重载".to_string());
        lines.join("\n")
    }
}

/// 执行 listen 命令
async fn exec_listen(state: &AppState, who: &str) -> String {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .input_tx
        .send(InputCommand::AddListen {
            who: who.to_string(),
            reply: reply_tx,
        })
        .await
        .is_err()
    {
        return "[warn] InputEngine 不可用".to_string();
    }
    match reply_rx.await {
        Ok(Ok(true)) => {
            // 持久化
            if let Some(ref path) = state.config_path {
                let mut list = state.wechat.get_listen_list().await;
                if !list.contains(&who.to_string()) {
                    list.push(who.to_string());
                }
                crate::config::save_listen_list(path, &list);
            }
            format!("[ok] 监听已添加: {who}")
        }
        Ok(Ok(false)) => format!("[warn] 添加失败: {who}"),
        Ok(Err(e)) => format!("[warn] 错误: {e}"),
        Err(_) => "[warn] actor 响应通道已关闭".to_string(),
    }
}

/// 执行 unlisten 命令
async fn exec_unlisten(state: &AppState, who: &str) -> String {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .input_tx
        .send(InputCommand::RemoveListen {
            who: who.to_string(),
            reply: reply_tx,
        })
        .await
        .is_err()
    {
        return "[warn] InputEngine 不可用".to_string();
    }
    match reply_rx.await {
        Ok(true) => {
            // 持久化
            if let Some(ref path) = state.config_path {
                let mut list = state.wechat.get_listen_list().await;
                list.retain(|n| n != who);
                crate::config::save_listen_list(path, &list);
            }
            format!("[ok] 监听已移除: {who}")
        }
        Ok(false) => format!("[warn] 未找到监听: {who}"),
        Err(_) => "[warn] actor 响应通道已关闭".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MsgContent;

    fn message(id: &str, chat: &str, sender: &str) -> DbMessage {
        DbMessage {
            message_id: id.to_string(),
            local_id: 1,
            server_id: 1,
            create_time: 1,
            content: "hello".to_string(),
            parsed: MsgContent::Text {
                text: "hello".to_string(),
            },
            msg_type: 1,
            talker: sender.to_string(),
            talker_display_name: sender.to_string(),
            chat: chat.to_string(),
            chat_display_name: chat.to_string(),
            is_self: false,
            is_at_me: false,
            at_user_list: Vec::new(),
            conversation_id: chat.to_string(),
            conversation_name: chat.to_string(),
            sender_id: sender.to_string(),
            sender_name: sender.to_string(),
            direction: "incoming".to_string(),
            is_group: false,
            attachment: None,
        }
    }

    #[tokio::test]
    async fn message_store_has_independent_cursors_and_filters() {
        let store = MessageStore::new(128);
        let first = store.push(message("m1", "chat-a", "user-a")).await;
        let second = store.push(message("m2", "chat-b", "user-b")).await;
        assert_eq!(first.cursor, 1);
        assert_eq!(second.cursor, 2);

        let (all, more) = store.query(0, 100, None, None, None).await;
        assert_eq!(all.len(), 2);
        assert!(!more);
        let (filtered, _) = store.query(0, 100, Some("chat-b"), None, None).await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message.message_id, "m2");
        let (after, _) = store.query(1, 100, None, None, None).await;
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].cursor, 2);
    }

    #[test]
    fn parses_http_byte_ranges() {
        let explicit = HeaderValue::from_static("bytes=10-19");
        assert_eq!(
            parse_range_header(Some(&explicit), 100).unwrap(),
            Some((10, 19))
        );
        let suffix = HeaderValue::from_static("bytes=-10");
        assert_eq!(
            parse_range_header(Some(&suffix), 100).unwrap(),
            Some((90, 99))
        );
        let invalid = HeaderValue::from_static("bytes=100-101");
        assert!(parse_range_header(Some(&invalid), 100).is_err());
    }
}

/// 执行 send 命令
async fn exec_send(state: &AppState, to: &str, text: &str) -> String {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let has_db = state.db.is_some();
    if state
        .input_tx
        .send(InputCommand::SendMessage {
            to: to.to_string(),
            text: text.to_string(),
            at: vec![],
            skip_verify: has_db,
            reply: reply_tx,
        })
        .await
        .is_err()
    {
        return "[warn] InputEngine 不可用".to_string();
    }
    match reply_rx.await {
        Ok(Ok((true, _, msg))) => format!("[ok] {msg}"),
        Ok(Ok((false, _, msg))) => format!("[warn] {msg}"),
        Ok(Err(e)) => format!("[warn] 发送失败: {e}"),
        Err(_) => "[warn] actor 响应通道已关闭".to_string(),
    }
}
