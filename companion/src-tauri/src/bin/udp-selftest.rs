//! UDP 链路独立自测（诊断脚手架，非产品代码）。
//!
//! **为什么需要它**：现象是 Companion 跑起来后模拟器一帧都收不到，而静态读
//! 代码看不出问题（`spawn_udp_thread` 明确在启动时 `send_command(CMD_QUERY_ALL)`，
//! `send_command` 的实现也标准）。这个 bin 把 `UdpLink` 从 Tauri / 线程 / 配置文件
//! 里完全剥出来，单独跑，判断问题出在链路本身还是宿主环境。
//!
//! 判定表：
//! - 本 bin 能发出帧 → 链路正常 → 问题在 Companion 宿主侧（配置读取/线程调度）
//! - 本 bin 也发不出 → 链路实现或运行环境有bug → 直接看本 bin 的错误输出
//!
//! 用法：`cargo run --bin udp-selftest`（默认打 127.0.0.1:17335）
//!       `cargo run --bin udp-selftest -- 192.168.1.10 17335`

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use companion_lib::constants::{ART_PI_PORT, CMD_HEARTBEAT, CMD_QUERY_ALL};
use companion_lib::log_buffer;
use companion_lib::v23::udp_client::UdpLink;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ip = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(ART_PI_PORT);

    let peer: SocketAddr = format!("{ip}:{port}")
        .parse()
        .unwrap_or_else(|e| panic!("peer 地址非法 {ip}:{port} — {e}"));

    println!("=== UDP 链路独立自测 ===");
    println!("targetpeer = {peer}");

    // 1) 能不能建链路
    let mut link = match UdpLink::new(peer) {
        Ok(l) => {
            println!("[OK] UdpLink::new 成功，本地 socket 已绑定");
            l
        }
        Err(e) => {
            println!("[FAIL] UdpLink::new 失败：{e}");
            std::process::exit(1);
        }
    };

    // 2) 启动查询（复刻 spawn_udp_thread 的第一行）
    match link.send_command(CMD_QUERY_ALL, &[], "查询全部设备") {
        Ok(()) => println!("[OK] 启动查询已发出（cmd=0x{:02X}）", CMD_QUERY_ALL),
        Err(e) => {
            println!("[FAIL] 启动查询发送失败：{e}");
            std::process::exit(1);
        }
    }

    // 3) 心跳必须换新seq（ART-Pi 有 16槽去重窗口，同 seq 会被丢）
    println!("--- 进入 pump循环（10s），观察 Ack / 状态帧 ---");
    let t0 = Instant::now();
    let mut acks = 0usize;
    let mut statuses = 0usize;

    while t0.elapsed() < Duration::from_secs(10) {
        for ev in link.pump(Duration::from_millis(50)) {
            match ev {
                companion_lib::v23::udp_client::LinkEvent::Acked { cmd, label } => {
                    acks += 1;
                    println!("[{:>5.1}s] ✅ Ack  cmd=0x{cmd:02X} {label}", t0.elapsed().as_secs_f64());
                }
                companion_lib::v23::udp_client::LinkEvent::Status(devs) => {
                    statuses += 1;
                    let s: Vec<String> = devs
                        .iter()
                        .map(|d| format!("#{}:{}{}", d.id, if d.onoff != 0 { "开" } else { "关" }, d.temp_x10 as f32 / 10.0))
                        .collect();
                    println!("[{:>5.1}s] 📊 状态帧 {}", t0.elapsed().as_secs_f64(), s.join(" "));
                }
                companion_lib::v23::udp_client::LinkEvent::LinkUp => {
                    println!("[{:>5.1}s] 🔗 链路已连通", t0.elapsed().as_secs_f64());
                }
                companion_lib::v23::udp_client::LinkEvent::LinkDown => {
                    println!("[{:>5.1}s] ⚠ 链路判活超时（3s 无回帧）", t0.elapsed().as_secs_f64());
                }
                companion_lib::v23::udp_client::LinkEvent::Failed { cmd, label } => {
                    println!("[{:>5.1}s] ❌ 重传耗尽 cmd=0x{cmd:02X} {label}", t0.elapsed().as_secs_f64());
                }
                companion_lib::v23::udp_client::LinkEvent::Log(m) => {
                    println!("[{:>5.1}s] · {m}", t0.elapsed().as_secs_f64());
                }
            }
        }
    }

    println!("\n=== 结果 ===");
    println!("Ack {acks} 条 / 状态帧 {statuses} 条 / 链路判定 up={}", link.is_up());
    let _ = log_buffer::snapshot();
    let _ = CMD_HEARTBEAT;
    if acks > 0 || statuses > 0 {
        println!("[结论] UdpLink 本身工作正常 ⇒ 问题在 Companion 宿主侧（配置/线程/事件循环）");
    } else {
        println!("[结论] UdpLink 在裸环境下也收不到回帧 ⇒ 查运行环境（防火墙/端口占用/IP 绑定）");
    }
}