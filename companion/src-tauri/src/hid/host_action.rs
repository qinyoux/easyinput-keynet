//! Host Action v1 监听器：持续读 0x11 输入报告，抽出 36 字节 UUID 交给桥接层。
//!
//! 与 easyop 的同名模块同构，但**只做分流不做执行**：easyop 拿到 UUID 后自己
//! 打开应用/切方案/注文本；Companion 只把 UUID 塞进通道，由 `home::bridge`
//! 翻译成家居命令。职责切分的目的：监听线程的节奏是"读到报文 → 交出 → 立刻
//! 回去读下一条"，一点都不能多（easyop 踩过的坑：把 6 秒的终端启动钉在监听
//! 线程里，用户看到的是"按了没反应"）。
//!
//! 报文布局（固件 `host_action_protocol.h`，并经 2026-10-15 实板 64B 报文逐字节核对）：
//! ```text
//! [0]    = 0x11          report_id
//! [1]    = 0x05          kind (HOST_ACTION)
//! [2]    = chunk_index   实测 0
//! [3]    = total_chunks  实测 1
//! [4]    = uuid_len      实测恒为 36 (0x24)
//! [5..41] = 36B UUID      规范小写 hex，**起点就是 5，中间没有保留字节**
//! [42..]  = 0 填充至 64B
//! ```
//!
//! 实测样本（UUID = `00000000-0000-4000-8000-000000000009`）：
//! `11 05 00 01 24 30 30 30 30 30 30 30 30 2D ...`——`[5]=0x30` 即 ASCII `'0'`，
//! 直接就是 UUID 首字符。所以 `start = 1 + HOST_ACTION_V1_HEADER_LEN = 5` 是对的，
//! 这里的 `HOST_ACTION_V1_HEADER_LEN = 4` 指「report_id 之后的那 4 字节」
//! （kind / chunk / chunks / uuid_len）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Foundation::HANDLE;

use crate::constants::*;
use crate::hid::device;
use crate::hid::transport;
use crate::log_buffer;

/// HID 在线标志（前端链路指示灯左半边）。
static HID_ONLINE: AtomicBool = AtomicBool::new(false);

/// EasyInput 开发板是否在线（枚举到匹配 VID/PID 且打开集合成功）。
pub fn hid_online() -> bool {
    HID_ONLINE.load(Ordering::Relaxed)
}

/// 从一条输入报告中解析 Host Action UUID；非 0x11/0x05 或长度不足返回 None。
///
/// 纯函数，单测覆盖全部拒绝路径（这是协议边界，容错必须显式）。
pub fn parse_host_action_uuid(report: &[u8]) -> Option<String> {
    if report.len() <= 1 + HOST_ACTION_V1_HEADER_LEN {
        return None;
    }
    if report[0] != REPORT_ID_APP_COMMAND || report[1] != KIND_HOST_ACTION {
        return None;
    }
    let start = 1 + HOST_ACTION_V1_HEADER_LEN;
    let end = start + HOST_ACTION_V1_UUID_LEN;
    if end > report.len() {
        return None;
    }
    let uuid = String::from_utf8_lossy(&report[start..end]).to_string();
    if !is_canonical_uuid(&uuid) {
        return None;
    }
    Some(uuid)
}

/// 规范 UUID 判定：36 字符、8-4-4-4-12、全小写 hex。
///
/// 固件 `config_payload.cpp` 对 `host_action:<uuid>` 只认规范小写 hex；
/// 主机侧若放宽，就会出现"固件收了下发、板上按键却报 UnknownAction"的错配。
pub fn is_canonical_uuid(uuid: &str) -> bool {
    let b = uuid.as_bytes();
    if b.len() != HOST_ACTION_V1_UUID_LEN {
        return false;
    }
    for (i, &c) in b.iter().enumerate() {
        let dash = matches!(i, 8 | 13 | 18 | 23);
        if dash {
            if c != b'-' {
                return false;
            }
        } else if !c.is_ascii_digit() && !(b'a'..=b'f').contains(&c) {
            return false;
        }
    }
    true
}

/// 打开所有匹配 VID/PID 且支持 Feature Report 的 HID 句柄（跳过键盘集合）。
fn try_open_all() -> Vec<HANDLE> {
    let mut handles = Vec::new();
    for path in device::enumerate_device_paths() {
        if let Some(handle) = device::open_matching(&path) {
            if device::has_feature_report(handle) {
                handles.push(handle);
            } else {
                device::close_handle(handle);
            }
        }
    }
    handles
}

/// 启动后台监听线程。`on_uuid` 在监听线程内被调用——**必须立即返回**，
/// 耗时动作（如发 UDP）由桥接层丢进通道后异步完成。
pub fn spawn_listener<F>(stop: Arc<AtomicBool>, mut on_uuid: F)
where
    F: FnMut(String) + Send + 'static,
{
    std::thread::spawn(move || listen_loop(&stop, &mut on_uuid));
}

fn listen_loop<F: FnMut(String)>(stop: &AtomicBool, on_uuid: &mut F) {
    log_buffer::info("Host Action 监听器已启动");
    let mut last_connected = false;
    while !stop.load(Ordering::Relaxed) {
        let handles = try_open_all();
        if handles.is_empty() {
            if last_connected {
                log_buffer::warn("监听器：未找到 HID 设备，等待连接…");
                last_connected = false;
                HID_ONLINE.store(false, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(1000));
            continue;
        }
        if !last_connected {
            log_buffer::info(format!(
                "监听器：已连接 HID 设备（枚举 {} 个匹配路径，打开 {} 个集合句柄）",
                device::enumerate_device_paths().len(),
                handles.len()
            ));
            last_connected = true;
            HID_ONLINE.store(true, Ordering::Relaxed);
        }
        let mut broken = false;
        while !stop.load(Ordering::Relaxed) {
            let mut got = false;
            for handle in handles.iter() {
                match transport::read_input_report_once(*handle, 50) {
                    Ok(Some(report)) => {
                        got = true;
                        if let Some(uuid) = parse_host_action_uuid(&report) {
                            log_buffer::info(format!("Host Action 触发 UUID={uuid}"));
                            on_uuid(uuid);
                        }
                    }
                    Ok(None) => continue,
                    Err(_) => {
                        broken = true;
                        break;
                    }
                }
            }
            if broken {
                log_buffer::warn("监听器：读取失败/设备断开，重连中");
                break;
            }
            if !got {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        for h in handles {
            device::close_handle(h);
        }
    }
    log_buffer::info("Host Action 监听器已停止");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条 0x11 kind 0x05 报文。
    ///
    /// 布局以固件 `host_action_protocol.cpp::encode_host_action_v1` 为真源：
    /// `[0]=report_id(0x11) [1]=kind(0x05) [2]=chunk_index [3]=total_chunks
    ///  [4]=uuid_len [5..41]=36B UUID`——即"4B 头"是 kind/chunk/chunks/len，
    ///  report_id 之外的那 4 字节，UUID 起点 = 1 + 4 = 5。
    fn report(uuid: &str) -> Vec<u8> {
        let mut r = vec![REPORT_ID_APP_COMMAND, KIND_HOST_ACTION, 0, 1, 36];
        r.extend_from_slice(uuid.as_bytes());
        r
    }

    #[test]
    fn parses_canonical_uuid() {
        let r = report("00000000-0000-4000-8000-000000000101");
        assert_eq!(
            parse_host_action_uuid(&r).as_deref(),
            Some("00000000-0000-4000-8000-000000000101")
        );
    }

    #[test]
    fn rejects_wrong_report_id() {
        let mut r = report("00000000-0000-4000-8000-000000000101");
        r[0] = 0x10; // 配置通道，不是回执通道
        assert!(parse_host_action_uuid(&r).is_none());
    }

    #[test]
    fn rejects_wrong_kind() {
        let mut r = report("00000000-0000-4000-8000-000000000101");
        r[1] = 0x03; // 配置回执
        assert!(parse_host_action_uuid(&r).is_none());
    }

    #[test]
    fn rejects_truncated_uuid() {
        let r = report("00000000-0000-4000-8000-00000000010"); // 35 字符
        assert!(parse_host_action_uuid(&r).is_none());
    }

    #[test]
    fn rejects_non_canonical_uuid() {
        // 大写 hex：固件侧 parse_action 不认。
        let r = report("00000000-0000-4000-8000-00000000010A");
        assert!(parse_host_action_uuid(&r).is_none());
        // 缺连字符。
        let r = report("0000000-0000-4000-8000-0000000000101");
        assert!(parse_host_action_uuid(&r).is_none());
    }

    #[test]
    fn canonical_uuid_checker() {
        assert!(is_canonical_uuid("00000000-0000-4000-8000-000000000101"));
        assert!(is_canonical_uuid("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"));
        assert!(!is_canonical_uuid("A1B2C3D4-E5F6-4A7B-8C9D-0E1F2A3B4C5D"));
        assert!(!is_canonical_uuid(""));
        assert!(!is_canonical_uuid("zzzzzzzz-0000-4000-8000-000000000101"));
        assert!(!is_canonical_uuid("00000000000040008000000000000101"));
    }
}
