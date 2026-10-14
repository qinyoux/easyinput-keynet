//! Tauri 命令层（前端 `invoke` 入口）。
//!
//! 原则：**命令只做"转交 + 返回结果"，不做业务**。业务在桥接层与场景机里，
//! 命令层负责把参数校验、锁顺序、通道投递做对。
//!
//! 锁顺序铁律：**map → mgr → link**，任何路径不得逆序（逆序是死友的充分条件）。

use tauri::{Manager, State};

use crate::config_store::HomeMap;
use crate::home::bridge::{self, OutCmd};
use crate::log_buffer;
use crate::state::{AppState, Snapshot};

/// 整机快照（前端启动与每次事件后调用）。
#[tauri::command]
pub fn get_snapshot(state: State<'_, AppState>) -> Snapshot {
    state.snapshot()
}

/// 历史日志（install 之前的事件前端收不到，启动时补拉）。
#[tauri::command]
pub fn get_logs() -> Vec<crate::log_buffer::LogEntry> {
    log_buffer::snapshot()
}

/// 界面点击场景按钮 → 与按键同一条代码路径（桥接层 dispatch）。
#[tauri::command]
pub fn activate_scene(scene: u8, state: State<'_, AppState>) -> Result<String, String> {
    let uuid = crate::constants::scene_uuid(scene)
        .ok_or_else(|| format!("场景 {scene} 不存在"))?
        .to_string();
    let map = state.map.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut mgr = state.mgr.lock().unwrap_or_else(|e| e.into_inner());
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    bridge::dispatch(&uuid, &map, &mut mgr, &tx)
}

/// 调试注入台：直接发一条 V2.3 命令（如 {"cmd":65,"payload":[1,1]}）。
///
/// 注入**不经过场景机**，本地设备视图等 Ack/状态帧回传后修正——
/// 权威状态永远以 ART-Pi 回传为准，本地乐观更新只发生在场景展开路径。
#[tauri::command]
pub fn send_raw_command(cmd: u8, payload: Vec<u8>) -> Result<String, String> {
    if payload.len() > crate::constants::FRAME_PAYLOAD_MAX {
        return Err(format!(
            "载荷越界: {}（上限 {}）",
            payload.len(),
            crate::constants::FRAME_PAYLOAD_MAX
        ));
    }
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    bridge::inject(&tx, cmd, &payload)
}

/// 立即查询全部设备状态。
#[tauri::command]
pub fn query_all() -> Result<String, String> {
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    tx.send(OutCmd::QueryAll)
        .map_err(|e| format!("命令通道已断开: {e}"))?;
    Ok("已发送全设备查询".into())
}

/// 修改 ART-Pi 地址并立即生效（UDP 线程换目标，旧待确认队列作废）。
#[tauri::command]
pub fn set_artpi_ip(ip: String, state: State<'_, AppState>) -> Result<String, String> {
    let ip = ip.trim().to_string();
    if ip.parse::<std::net::IpAddr>().is_err() {
        return Err(format!("不是合法 IP: {ip}"));
    }
    {
        let mut map = state.map.lock().unwrap_or_else(|e| e.into_inner());
        map.artpi_ip = ip.clone();
    }
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    tx.send(OutCmd::SetPeer(ip.clone()))
        .map_err(|e| format!("命令通道已断开: {e}"))?;
    Ok(format!("ART-Pi 地址已切换为 {ip}"))
}

/// 保存配置到磁盘（配置页「保存」按钮）。
#[tauri::command]
pub fn save_map(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("定位配置目录失败: {e}"))?;
    let map = state.map.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let path = crate::config_store::save(&dir, &map)?;
    // 场景表若被改过，同步进状态机（保留设备状态不重置）。
    let mut mgr = state.mgr.lock().unwrap_or_else(|e| e.into_inner());
    mgr.replace_scenes(map.scenes.clone());
    Ok(format!("配置已保存到 {}", path.display()))
}

/// 读取当前配置（配置页展示）。
#[tauri::command]
pub fn get_map(state: State<'_, AppState>) -> HomeMap {
    state
        .map
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 整份替换配置并落盘（配置页「读-改-存」的写入口）。
///
/// 场景表同步进状态机；地址变更同时通知 UDP 线程换目标。
#[tauri::command]
pub fn update_map(
    map: HomeMap,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("定位配置目录失败: {e}"))?;
    let mut sanitized = map;
    crate::config_store::sanitize_pub(&mut sanitized);
    {
        let mut guard = state.map.lock().unwrap_or_else(|e| e.into_inner());
        *guard = sanitized.clone();
    }
    {
        let mut mgr = state.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.replace_scenes(sanitized.scenes.clone());
    }
    let path = crate::config_store::save(&dir, &sanitized)?;
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    tx.send(OutCmd::SetPeer(sanitized.artpi_ip.clone()))
        .map_err(|e| format!("命令通道已断开: {e}"))?;
    Ok(format!("配置已保存到 {}", path.display()))
}

/// 单设备开关（设备卡片上的按钮，与按键同路径）。
#[tauri::command]
pub fn toggle_device(device: u8, state: State<'_, AppState>) -> Result<String, String> {
    let mut mgr = state.mgr.lock().unwrap_or_else(|e| e.into_inner());
    let tx = crate::out_tx().map_err(|_| "UDP 线程未就绪".to_string())?;
    let (cmd, payload, label) = mgr.toggle_device(device)?;
    tx.send(OutCmd::Send {
        cmd,
        payload,
        label: label.clone(),
    })
    .map_err(|e| format!("命令通道已断开: {e}"))?;
    Ok(label)
}
