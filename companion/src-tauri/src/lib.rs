//! EasyInput Companion 应用装配。
//!
//! 线程模型（AppHandle 全程只 clone 一次，见 `[patch] tao` 说明）：
//! ```text
//! 主线程      Tauri 命令 / 事件循环
//!   │
//!   ├─ HID 监听线程   读 0x11 报告 → UUID → bridge::dispatch → OutCmd 通道
//!   │                  （不碰 AppHandle，不等 IO）
//!   └─ UDP 链路线程   持 UdpLink + 通道接收端：收发/重传/心跳/判活
//!                      （唯一持有 AppHandle clone 的后台线程，负责
//!                       状态更新与事件广播）
//! ```

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::config_store::HomeMap;
use crate::constants::*;
use crate::home::bridge::OutCmd;
use crate::state::AppState;
use crate::v23::udp_client::{LinkEvent, UdpLink};

pub mod commands;
pub mod config_store;
pub mod constants;
pub mod crc16;
pub mod hid;
pub mod home;
pub mod log_buffer;
pub mod state;
pub mod v23;

/// 前端快照事件名（设备/链路/场景变化广播）。
pub const EVENT_SNAPSHOT: &str = "companion-snapshot";

/// UDP 线程命令通道（Tauri 命令层投递用）。
static OUT_TX: OnceLock<Sender<OutCmd>> = OnceLock::new();

/// 取命令通道发送端。未就绪返回 Err（前端会看到"UDP 线程未就绪"）。
pub fn out_tx() -> Result<Sender<OutCmd>, String> {
    OUT_TX.get().cloned().ok_or_else(|| "UDP 线程未就绪".to_string())
}

/// HID 监听线程停止标志（窗口关闭时置位）。
static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            log_buffer::install_app_handle(&handle);
            let _ = APP_STATE_HANDLE.set(handle.clone());

            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("定位配置目录失败: {e}"))?;
            let map: HomeMap = config_store::load(&config_dir);
            let peer = parse_peer(&map);

            app.manage(AppState::new(map.clone(), peer.to_string()));

            // ---- 命令通道 ----
            let (tx, rx) = mpsc::channel::<OutCmd>();
            let _ = OUT_TX.set(tx.clone());

            // ---- UDP 链路线程（唯一持有 AppHandle clone 的后台线程）----
            spawn_udp_thread(handle.clone(), rx, peer);

            // ---- HID 监听线程 ----
            let stop = Arc::new(AtomicBool::new(false));
            let _ = STOP.set(stop.clone());
            let tx_hid = tx.clone();
            hid::host_action::spawn_listener(stop, move |uuid| {
                // 监听线程纪律：取出 State → 派发 → 立刻返回，绝不在回调里做 IO。
                if let Some(h) = APP_STATE_HANDLE.get() {
                    if let Some(st) = h.try_state::<AppState>() {
                        let map = st
                            .map
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        let mut mgr = st.mgr.lock().unwrap_or_else(|e| e.into_inner());
                        if let Err(e) = home::bridge::dispatch(&uuid, &map, &mut mgr, &tx_hid) {
                            log_buffer::warn(format!("动作执行失败: {e}"));
                        }
                    }
                }
                emit_snapshot();
            });

            // ---- 窗口关闭：优雅停止后台线程 ----
            let stop2 = STOP.get().cloned();
            if let Some(win) = app.get_webview_window("main") {
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        if let Some(s) = &stop2 {
                            s.store(true, Ordering::Relaxed);
                        }
                    }
                });
            }

            log_buffer::info(format!(
                "Companion 启动完成：ART-Pi 目标 {}，场景模式 {}",
                peer, map.scene_mode
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_logs,
            commands::activate_scene,
            commands::send_raw_command,
            commands::query_all,
            commands::set_artpi_ip,
            commands::save_map,
            commands::get_map,
            commands::update_map,
            commands::toggle_device,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 运行失败");
}

/// 内部占位模块（避免 lib_prelude 名误导；保留给未来预导入用）。
mod lib_prelude_hidden {}
#[path = ""]
mod lib_prelude {}

/// 主线程持有的 AppHandle 副本（供 HID 回调跨线程取 State）。
///
/// ⚠️ 这是**唯一**允许的额外 clone：tao UB 补丁已消除并发 clone 的崩溃面，
/// 但纪律上仍保持"最少 clone"——UDP 线程一个、这里一个、其余一律经
/// `try_state` / 命令参数获取。
static APP_STATE_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// 广播整机快照（任意线程可调；无窗口时静默）。
pub fn emit_snapshot() {
    if let Some(app) = APP_STATE_HANDLE.get() {
        if let Some(st) = app.try_state::<AppState>() {
            let _ = app.emit(EVENT_SNAPSHOT, st.snapshot());
        }
    }
}

fn parse_peer(map: &HomeMap) -> SocketAddr {
    let ip = map
        .artpi_ip
        .parse::<std::net::IpAddr>()
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 10)));
    SocketAddr::new(ip, map.artpi_port)
}

/// UDP 链路线程主循环。
fn spawn_udp_thread(app: AppHandle, rx: Receiver<OutCmd>, peer: SocketAddr) {
    std::thread::spawn(move || {
        let mut link = match UdpLink::new(peer) {
            Ok(l) => l,
            Err(e) => {
                log_buffer::err(format!("UDP 链路启动失败: {e}"));
                return;
            }
        };
        log_buffer::info(format!("UDP 链路已启动，目标 {peer}（:{}）", ART_PI_PORT));
        // 启动即查一次全设备状态，让前端首屏有数据。
        let _ = link.send_command(CMD_QUERY_ALL, &[], "查询全部设备");

        let mut last_emit = Instant::now();
        let mut dirty = true;
        loop {
            // 1. 命令通道排空。
            let mut peer_changed = false;
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    OutCmd::Send { cmd, payload, label } => {
                        if let Err(e) = link.send_command(cmd, &payload, &label) {
                            log_buffer::err(format!("发送失败（{label}）: {e}"));
                        }
                        dirty = true;
                    }
                    OutCmd::SetPeer(ip) => {
                        let port = app
                            .try_state::<AppState>()
                            .map(|st| {
                                st.map
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .artpi_port
                            })
                            .unwrap_or(ART_PI_PORT);
                        if let Ok(addr) = format!("{ip}:{port}").parse::<SocketAddr>() {
                            link.set_peer(addr);
                            peer_changed = true;
                        }
                    }
                    OutCmd::QueryAll => {
                        let _ = link.send_command(CMD_QUERY_ALL, &[], "查询全部设备");
                        dirty = true;
                    }
                }
            }
            if peer_changed {
                let _ = link.send_command(CMD_QUERY_ALL, &[], "查询全部设备");
                dirty = true;
            }

            // 2. 周期事务：收包 / 重传 / 心跳 / 判活。
            for ev in link.pump(Duration::from_millis(50)) {
                match ev {
                    LinkEvent::Log(s) => log_buffer::info(s),
                    LinkEvent::Acked { cmd, label } => {
                        log_buffer::info(format!("ART-Pi 已确认 {label}（cmd=0x{cmd:02X}）"));
                        dirty = true;
                    }
                    LinkEvent::Failed { cmd, label } => {
                        log_buffer::err(format!(
                            "命令失败（重传 {} 次未确认）: {label}（cmd=0x{cmd:02X}）——检查 ART-Pi 是否在线、IP 是否正确",
                            RETRANSMIT_MAX
                        ));
                        dirty = true;
                    }
                    LinkEvent::Status(states) => {
                        if let Some(st) = app.try_state::<AppState>() {
                            let mut mgr = st.mgr.lock().unwrap_or_else(|e| e.into_inner());
                            mgr.apply_status(&states);
                        }
                        dirty = true;
                    }
                    LinkEvent::LinkUp => {
                        log_buffer::info("链路已连通（ART-Pi 在线）");
                        let _ = link.send_command(CMD_QUERY_ALL, &[], "查询全部设备");
                        dirty = true;
                    }
                    LinkEvent::LinkDown => {
                        log_buffer::warn("链路中断（3s 未收到 ART-Pi 回帧）——命令将进入重传");
                        dirty = true;
                    }
                }
            }

            // 3. 链路信息同步 + 节流广播。
            if let Some(st) = app.try_state::<AppState>() {
                let mut info = st.link.lock().unwrap_or_else(|e| e.into_inner());
                let up = link.is_up();
                let pending = link.pending_len();
                if info.up != up || info.pending != pending {
                    info.up = up;
                    info.pending = pending;
                    dirty = true;
                }
                let peer_now = link.peer().to_string();
                if info.peer != peer_now {
                    info.peer = peer_now;
                }
            }
            if dirty && last_emit.elapsed() >= Duration::from_millis(300) {
                emit_snapshot();
                last_emit = Instant::now();
                dirty = false;
            }
        }
    });
}
