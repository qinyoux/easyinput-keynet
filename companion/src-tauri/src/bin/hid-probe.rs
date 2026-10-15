//! HID 探测器：把开发板发出的**每一条** 0x11 输入报告原样打出来。
//!
//! 用途：联调第一步的"见真数据"。之前 Companion 的监听器只挑 kind 0x05（Host
//! Action）并把其余报文静默丢掉，于是"按了键没反应"这件事无法区分三种原因：
//!
//!   1. 板子根本没发报文（该键没绑 host_action，或固件没跑起来）；
//!   2. 板子发了但 kind 不是 0x05（走了语音/普通按键路径）；
//!   3. 板子发了 kind 0x05 但 UUID 不规范（主机侧解析拒绝）。
//!
//! 这个工具**不做任何过滤**：所有报文全打，附上 kind 名称与 UUID 解析结果，
//! 三种情况一眼分开。真源参照：easyop `constants.rs`（kind 0x03/0x04/0x05）。
//!
//! 用法：
//!   cargo run --bin hid-probe
//!   （板子插上后，逐个按 KEY1–KEY8 与编码器左旋/右旋/按下）
//!
//! 退出：Ctrl-C。

use std::time::Instant;

use companion_lib::constants::{
    HOST_ACTION_V1_HEADER_LEN, HOST_ACTION_V1_UUID_LEN, PID, REPORT_ID_APP_COMMAND, VID,
};
use companion_lib::hid::device;
use companion_lib::hid::host_action::parse_host_action_uuid;
use companion_lib::hid::transport;

/// 0x11 kind → 名称（真源：easyop `constants.rs`）。
fn kind_name(kind: u8) -> &'static str {
    match kind {
        0x03 => "CONFIG_ACK(配置回执)",
        0x04 => "STATUS_RESPONSE(状态响应)",
        0x05 => "HOST_ACTION(按键绑定触发)",
        _ => "未知 kind",
    }
}

/// 把一条报告按 kind 布局拆开打印。
///
/// 0x11 的公共布局：`[0]=report_id [1]=kind [2..6]=4B header [6..]=payload`。
/// kind 0x05 的 header 是 `chunk_index / total_chunks / uuid_len`，payload 是 36B UUID。
fn describe(report: &[u8]) -> String {
    if report.is_empty() {
        return "空报文".to_string();
    }
    if report[0] != REPORT_ID_APP_COMMAND {
        return format!("[0x{:02X}] 非 0x11 通道（可能是键盘集合的普通按键报告）", report[0]);
    }
    if report.len() < 2 {
        return "[0x11] 只有 report_id，无 kind".to_string();
    }
    let kind = report[1];
    let head = format!(
        "[0x{:02X}] kind=0x{:02X} {}", report[0], kind, kind_name(kind)
    );

    if report.len() < 2 + HOST_ACTION_V1_HEADER_LEN {
        return format!("{head} —— 长度不足（{}B，缺 header）", report.len());
    }
    // header 4B = [2]=chunk_index [3]=total_chunks [4]=uuid_len [5]=保留
    let chunk = report[2];
    let chunks = report[3];
    let uuid_len = report[4];

    if kind == 0x05 {
        // 实板 64B 报文核对结论：UUID 起点 = 5（[5] 直接是 ASCII '0'，无保留字节）。
        // 这里仍打印 [5..41] 与 [6..42] 对照，作为该结论的可复现证据。
        let at5 = String::from_utf8_lossy(&report[5..(5 + HOST_ACTION_V1_UUID_LEN).min(report.len())]);
        let at6 = String::from_utf8_lossy(&report[6..(6 + HOST_ACTION_V1_UUID_LEN).min(report.len())]);
        let parsed = parse_host_action_uuid(report);
        let verdict = match parsed {
            Some(u) => format!("✅ 解析通过：{u}"),
            None => {
                if uuid_len as usize != HOST_ACTION_V1_UUID_LEN {
                    format!(
                        "❌ uuid_len={uuid_len}（期望 {}）—— 长度字段就不对",
                        HOST_ACTION_V1_UUID_LEN
                    )
                } else {
                    "❌ 起点 5 取不出规范 UUID（与实测结论矛盾，需复查 raw）".to_string()
                }
            }
        };
        format!(
            "{head} | chunk={chunk}/{chunks} uuid_len={uuid_len}\n             起点5 [5..41]=\"{}\"\n             起点6 [6..42]=\"{}\"  ← 少首字符，仅作对照\n           {verdict}",
            at5.trim_end_matches('\0'),
            at6.trim_end_matches('\0')
        )
    } else {
        format!("{head} | header=[{} {} {} {}] | 非 host_action，按键走的是其它路径",
            chunk, chunks, uuid_len, report[5])
    }
}

fn main() {
    println!("========================================================");
    println!(" EasyInput HID 探测器  (VID {VID:04X} / PID {PID:04X})");
    println!(" 打印开发板发出的每一条 0x11 报告，不过滤、不丢弃。");
    println!(" 现在开始逐个按板子上的 KEY1–KEY8 和编码器左旋/右旋/按下。");
    println!(" 看完直接关掉这个窗口即可退出。");
    println!("========================================================");

    let start = Instant::now();
    let mut connected = false;

    loop {
        // 打开所有支持 Feature Report 的匹配集合（跳过键盘集合 COL01）。
        let mut handles = Vec::new();
        for path in device::enumerate_device_paths() {
            if let Some(h) = device::open_matching(&path) {
                if device::has_feature_report(h) {
                    handles.push(h);
                } else {
                    device::close_handle(h);
                }
            }
        }

        if handles.is_empty() {
            if connected {
                println!("--- 设备断开，等待重连…");
                connected = false;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(800));
                continue;
            }
        }

        if !connected {
            println!(
                "\n[{:>7.1}s] ✅ 已连接：打开 {} 个 Feature Report 集合句柄（已跳过键盘集合）",
                start.elapsed().as_secs_f64(),
                handles.len()
            );
            println!("           现在按板子上的键，每次按完等 1 秒。\n");
            connected = true;
        }

        // 读一轮：轮询所有句柄，任何一条报告都打印。
        let mut got_any = false;
        let mut broken = false;
        for h in &handles {
            match transport::read_input_report_once(*h, 100) {
                Ok(Some(report)) => {
                    got_any = true;
                    // 打印前 48 字节原始 hex —— UUID 偏移争议只能靠真数据定案，
                    // 不能靠推导（2026-10-15 教训：注释与实测不符时，先看字节）。
                    let head_hex: Vec<String> =
                        report.iter().take(48).map(|b| format!("{b:02X}")).collect();
                    println!(
                        "[{:>7.1}s] {}B  raw[0..48]: {}",
                        start.elapsed().as_secs_f64(),
                        report.len(),
                        head_hex.join(" ")
                    );
                    println!("           → {}", describe(&report));
                }
                Ok(None) => continue,
                Err(e) => {
                    println!("[{:>7.1}s] ⚠ 读取失败：{e}（设备可能拔了）", start.elapsed().as_secs_f64());
                    broken = true;
                    break;
                }
            }
        }
        for h in handles {
            device::close_handle(h);
        }
        if broken {
            std::thread::sleep(std::time::Duration::from_millis(500));
            continue;
        }
        if !got_any {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
