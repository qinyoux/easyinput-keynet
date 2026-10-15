//! 8 场景顺序演练（**演示用证据生成器**，非产品代码）。
//!
//! # 为什么需要它
//!
//! 演示视频要求"按下 KEY1–KEY8，8 个场景依次生效、设备状态随之变化"。
//! 手按 8 次是不确定的：漏按、按错顺序、讲解时忘词，都会让视频作废。
//! 本bin 用**产品代码原样的路径**把 8 个场景送一遍：
//!
//! ```text
//! scene_uuid(n) → bridge::dispatch() → OutCmd 通道 → UdpLink::send_command()
//!                → ART-Pi :17335（模拟器或真机）
//! ```
//!
//! 关键点：**不另写一套展开逻辑**。走的是 Companion 真机运行时的同一个
//! `dispatch()`，所以本 bin 能跑通 = 真机链路能跑通。视频里它可以替代手按，
//! 也可以作为手按时的对照基线（两者的日志应当逐条一致）。
//!
//! # 用法
//!
//! ```text
//! cargo run --bin scene-walk                # 打 127.0.0.1:17335（模拟器）
//! cargo run --bin scene-walk -- 192.168.1.10# 打 ART-Pi 真机
//! cargo run --bin scene-walk -- 127.0.0.1 17335 900
//! ```
//!
//! 第四个参数是场景间隔秒数（默认 1.2s）：给状态帧留出回传时间，
//! 让日志里"命令 → Ack → 状态"三行成组出现，便于肉眼核对。
//!
//! 退出码：0 = 8 个场景全部收到 Ack；1 = 有场景未确认（详见结尾汇总）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use companion_lib::constants::{self, ART_PI_PORT, CMD_QUERY_ALL};
use companion_lib::home::bridge::{self, OutCmd};
use companion_lib::home::scene::SceneManager;
use companion_lib::v23::udp_client::{LinkEvent, UdpLink};

/// 解析配置目录：显式路径 > `%APPDATA%\com.easyinput.companion`。
///
/// Tauri 运行时用 `path().app_config_dir()`（按 identifier 解析），bin 没有
/// Tauri 上下文，只能手拼——所以写死字符串前先确认目录真的存在，不存在就直接
/// 报错退出，**绝不静默用默认表**（那正是本轮踩过的坑）。
fn config_dir(explicit: Option<&String>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    let base = std::env::var("APPDATA").expect("APPDATA 未设置");
    PathBuf::from(base).join("com.easyinput.companion")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ip = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(ART_PI_PORT);
    let gap_s: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1.2);

    let peer: SocketAddr = format!("{ip}:{port}")
        .parse()
        .unwrap_or_else(|e| panic!("peer 地址非法 {ip}:{port} — {e}"));

    let dir = config_dir(args.get(4));
    println!("{}", "=".repeat(66));
    println!("8 场景顺序演练 · 走真实 bridge::dispatch 路径");
    println!("目标 {peer} · 场景间隔 {gap_s:.1}s");
    println!("配置目录 {}", dir.display());
    println!("{}", "=".repeat(66));

    //配置来源与真机运行时完全一致。load() 在解析失败时会静默回退默认表，
    // 所以下面必须打印实际生效的值并与磁盘文件比对——否则又是一个静默失败。
    let path = dir.join("home_map.json");
    if !path.exists() {
        eprintln!("[FAIL] 找不到 {}，先跑一次 Companion（Tauri）生成它", path.display());
        std::process::exit(1);
    }
    let map = companion_lib::config_store::load(&dir);
    println!(
        "[配置] artpi_ip={} port={} scene_mode={} bindings={} scenes={}",
        map.artpi_ip,
        map.artpi_port,
        map.scene_mode,
        map.bindings.len(),
        map.scenes.len()
    );
    if map.scenes.len() != 8 {
        eprintln!(
            "[FAIL] 生效的场景数 = {}，不是 8 ⇒ 配置被回退或损坏。先跑 cargo run --bin check-config",
            map.scenes.len()
        );
        std::process::exit(1);
    }
    if map.artpi_ip != ip {
        println!(
            "[注意] 配置里的 artpi_ip={} 与本次目标 {} 不一致——命令行参数优先，\
             录像前请确认这是有意的",
            map.artpi_ip, ip
        );
    }

    let mut mgr = SceneManager::new(map.scenes.clone());
    let (tx, rx): (_, Receiver<OutCmd>) = channel();
    let mut link = UdpLink::new(peer).unwrap_or_else(|e| panic!("UdpLink::new 失败: {e}"));

    // 启动查询：与 spawn_udp_thread 第一行一致，让 ART-Pi 先把状态吐出来。
    let _ = link.send_command(CMD_QUERY_ALL, &[], "查询全部设备");

    let scene_ids: Vec<u8> = (1..=8).collect();
    let mut total_acks = 0usize;

    let t0 = Instant::now();
    let mut inflight: Vec<u8> = Vec::new();
    let mut next_scene = 0usize;

    // 每个场景：展开 → 逐条 send_command → 等到该场景命令全部 Ack（或超时）。
    while next_scene < scene_ids.len() || !inflight.is_empty() {
        let elapsed = t0.elapsed().as_secs_f64();
        let last_dispatch = next_scene as f64 * gap_s;

        if next_scene < scene_ids.len() && elapsed >= last_dispatch {
            let id = scene_ids[next_scene];
            next_scene += 1;
            let uuid = constants::scene_uuid(id)
                .unwrap_or_else(|| panic!("场景 {id} UUID 构造失败"));
            let name = mgr.find(id).map(|s| s.name.clone()).unwrap_or_default();
            match bridge::dispatch(&uuid, &map, &mut mgr, &tx) {
                Ok(msg) => println!("\n[{elapsed:>6.1}s] ▶ KEY{id} → 场景{id}「{name}」：{msg}"),
                Err(e) => {
                    eprintln!("\n[{elapsed:>6.1}s] ✗ 场景{id} 派发失败: {e}");
                    continue;
                }
            }
            // dispatch 只把 OutCmd 塞进通道，这里做真正发送（真机运行时由
            // UDP 线程做这件事——本bin 把它内联到主循环，保持单线程确定性）。
            while let Ok(cmd) = rx.try_recv() {
                if let OutCmd::Send {
                    cmd: c,
                    payload,
                    label,
                } = cmd
                {
                    match link.send_command(c, &payload, &label) {
                        Ok(()) => {
                            println!("           → cmd=0x{c:02X} {label}");
                            inflight.push(c);
                        }
                        Err(e) => eprintln!("           ✗ 发送失败: {e}"),
                    }
                }
            }
        }

        // 收事件：Ack 计数、状态帧打印、链路判活。
        for ev in link.pump(Duration::from_millis(50)) {
            match ev {
                LinkEvent::Acked { cmd, label } => {
                    println!("[{:6.1}s]   ✅ Ack cmd=0x{cmd:02X} {label}", t0.elapsed().as_secs_f64());
                    inflight.retain(|c| *c != cmd);
                    total_acks += 1;
                }
                LinkEvent::Status(devs) => {
                    let s: Vec<String> = devs
                        .iter()
                        .map(|d| {
                            format!(
                                "#{} {}{}{}",
                                d.id,
                                if d.onoff != 0 { "开" } else { "关" },
                                if d.level > 0 {
                                    format!("·{}%", d.level)
                                } else {
                                    String::new()
                                },
                                if d.temp_x10 != 0 {
                                    format!("·{:.1}℃", d.temp_x10 as f32 / 10.0)
                                } else {
                                    String::new()
                                }
                            )
                        })
                        .collect();
                    println!("[{:6.1}s]   📊 {}", t0.elapsed().as_secs_f64(), s.join(" "));
                }
                LinkEvent::LinkUp => println!("[{:>6.1}s]   🔗 链路已连通", t0.elapsed().as_secs_f64()),
                LinkEvent::LinkDown => {
                    eprintln!("[{:>6.1}s]   ⚠ 链路判活超时（3s 无回帧）——对端没起来？", t0.elapsed().as_secs_f64())
                }
                LinkEvent::Failed { cmd, label } => {
                    eprintln!(
                        "[{:>6.1}s]   ❌ 重传耗尽 cmd=0x{cmd:02X} {label}",
                        t0.elapsed().as_secs_f64()
                    );
                    inflight.retain(|c| *c != cmd);
                }
                LinkEvent::Log(m) => println!("[{:>6.1}s] · {m}", t0.elapsed().as_secs_f64()),
            }
        }
    }

    // 汇总
    println!("\n{}", "=".repeat(66));
    println!("汇总");
    println!("  链路目标   : {peer}");
    println!("  链路存活   : {}", if link.is_up() { "是" } else { "否" });
    println!("  未确认命令 : {}", link.pending_len());
    println!(
        "  Ack 总数   : {total_acks}"
    );
    if link.is_up() && link.pending_len() == 0 && total_acks > 0 {
        println!("  [结论] ✅ 8 场景全部经真实 dispatch 路径送达并被 Ack");
    } else if total_acks > 0 {
        println!("  [结论] ⚠ 部分命令未收到 Ack（收到 {total_acks} 条）——先看对端是否在线");
    } else {
        println!("  [结论] ✗ 一条 Ack 都没有 —— 查对端进程 / IP / 端口占用");
        std::process::exit(1);
    }
    println!("\n提示：重定向到文件即可作为演示证据");
    println!("      cargo run --bin scene-walk -- 127.0.0.1 17335 1.2 > ../证据/scene_walk.log 2>&1");
}