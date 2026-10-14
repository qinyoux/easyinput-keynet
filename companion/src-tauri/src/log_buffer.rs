//! 日志环形缓冲 + 前端事件转发。
//!
//! 后台线程（HID 监听 / UDP 链路）没有 Tauri `State` 可拿，只有 `AppHandle`
//! 能发事件；而 `AppHandle` 在 `setup` 之后才存在。因此用 `OnceLock` 存一份
//! clone，线程内只调 `log(info!(...))` 宏风格的函数，不直接碰 AppHandle。
//!
//! 缓冲上限 500 条：前端"事件日志"面板够翻，又不至于让内存无界增长。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 前端事件名（`listen("companion-log", ...)`）。
pub const EVENT_LOG: &str = "companion-log";

const LOG_CAPACITY: usize = 500;

#[derive(Clone, Serialize)]
pub struct LogEntry {
    /// 毫秒 Unix 时间戳。
    pub ts: u64,
    /// "info" | "warn" | "err"。
    pub level: String,
    pub message: String,
}

static LOGS: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static APP: OnceLock<AppHandle> = OnceLock::new();

/// 在 `setup` 里调用一次，之后任意线程可写日志。
pub fn install_app_handle(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push(level: &str, message: String) {
    let entry = LogEntry {
        ts: now_ms(),
        level: level.to_string(),
        message,
    };
    {
        let mut guard = LOGS.lock().unwrap_or_else(|e| e.into_inner());
        if guard.len() >= LOG_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(entry.clone());
    }
    if let Some(app) = APP.get() {
        let _ = app.emit(EVENT_LOG, entry);
    }
}

pub fn info(message: impl Into<String>) {
    push("info", message.into());
}

pub fn warn(message: impl Into<String>) {
    push("warn", message.into());
}

#[allow(dead_code)]
pub fn err(message: impl Into<String>) {
    push("err", message.into());
}

/// 前端启动时拉取历史日志（事件只能收到 install 之后的）。
pub fn snapshot() -> Vec<LogEntry> {
    LOGS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}
