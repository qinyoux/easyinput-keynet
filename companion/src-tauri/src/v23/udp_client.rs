//! UDP :17335 链路：V2.3 帧收发、Ack 驱动的重传队列、心跳保活、状态解析。
//!
//! 结构分成两层，**IO 与策略分离**：
//! - [`RetransmitQueue`]：纯逻辑（无 socket、无时钟依赖之外的 IO），
//!   单测覆盖"200ms 重发 / 最多 3 次 / **重传必须换 seq**"；
//! - [`UdpLink`]：把队列粘到 `UdpSocket` 上，一个 `pump()` 做完全部周期事务
//!   （收包 → 派发 → 重传 → 心跳 → 链路判活）。
//!
//! # 为什么重传必须换 seq（本工程最隐蔽的坑）
//!
//! ART-Pi `sh_frame_seq_fresh` 用 16 槽位图记最近 16 个 seq，**窗口内同 seq
//! 直接丢弃**。若 PC 侧重传沿用原 seq，重传帧会被当成重放吃掉 ⇒ 命令永久
//! 丢失，现象是"日志显示发了、ART-Pi 就是没动作"。因此队列每次重传都向
//! 链路申请一个**全新 seq**（见 `poll()` 的 `alloc` 回调）。

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::log_buffer;
use crate::v23::frame::{self, Frame};

/// 重传扫描的一次结果。
pub enum Due {
    /// 需要（重）发送的帧字节。
    Retransmit(Vec<u8>),
    /// 重传次数用尽仍未确认，彻底失败。
    Failed(Pending),
}

/// 待确认的命令。
#[derive(Clone, Debug)]
pub struct Pending {
    pub cmd: u8,
    pub payload: Vec<u8>,
    pub sid: u8,
    /// 最近一次发送使用的 seq（Ack 按它匹配）。
    pub seq: u16,
    /// 已发送次数（含首送）。
    pub tries: u8,
    /// 最近一次发送时刻。
    pub sent_at: Instant,
    /// 人类可读描述，仅日志用。
    pub label: String,
}

/// Ack 驱动的重传队列（纯逻辑）。
#[derive(Default)]
pub struct RetransmitQueue {
    pending: Vec<Pending>,
}

impl RetransmitQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条已首发的命令。
    pub fn track(&mut self, cmd: u8, payload: Vec<u8>, sid: u8, seq: u16, label: String, now: Instant) {
        self.pending.push(Pending {
            cmd,
            payload,
            sid,
            seq,
            tries: 1,
            sent_at: now,
            label,
        });
    }

    /// 收到 Ack：按 seq 精确匹配，命中则移除并返回该命令。
    pub fn on_ack(&mut self, seq: u16) -> Option<Pending> {
        let idx = self.pending.iter().position(|p| p.seq == seq)?;
        Some(self.pending.remove(idx))
    }

    /// 到期扫描。`alloc` 由链路提供，为重传分配**全新 seq**。
    pub fn poll(&mut self, now: Instant, alloc: &mut impl FnMut() -> u16) -> Vec<Due> {
        let mut out = Vec::new();
        let mut failed_idx: Vec<usize> = Vec::new();
        for (i, p) in self.pending.iter_mut().enumerate() {
            if now.duration_since(p.sent_at) < Duration::from_millis(RETRANSMIT_AFTER_MS) {
                continue;
            }
            if p.tries <= RETRANSMIT_MAX {
                // 还有重传预算：换新 seq 重发（ART-Pi 去重窗口会吃同 seq 帧）。
                p.seq = alloc();
                p.tries += 1;
                p.sent_at = now;
                if let Ok(bytes) = frame::encode(&Frame {
                    frame_type: FT_CMD,
                    cmd: p.cmd,
                    seq: p.seq,
                    sid: p.sid,
                    payload: p.payload.clone(),
                }) {
                    out.push(Due::Retransmit(bytes));
                }
            } else {
                failed_idx.push(i);
            }
        }
        // 倒序移除，避免索引位移。
        for i in failed_idx.into_iter().rev() {
            out.push(Due::Failed(self.pending.remove(i)));
        }
        out
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// 链路一次 `pump` 产生的事件（由调用方转成日志 / 前端事件 / 状态更新）。
///
/// `Debug` 是测试断言的硬需求（失败时要打出事件列表），不是可选的。
#[derive(Debug)]
pub enum LinkEvent {
    Log(String),
    /// 命令被 ART-Pi 确认。
    Acked { cmd: u8, label: String },
    /// 命令重传次数用尽，彻底失败。
    Failed { cmd: u8, label: String },
    /// 收到状态（Ack 携带或周期状态帧）：若干设备条目。
    Status(Vec<DeviceState>),
    LinkUp,
    LinkDown,
}

/// 单设备运行状态（与 ART-Pi `sh_dev_state_t` 的协议视图对齐）。
///
/// 这是**全工程唯一的 DeviceState 类型**（`home::scene` 再导出）：链路回传的
/// 权威状态与本地乐观状态必须同型，否则每次转换都是一处潜伏的字段错位。
/// serde 派生是因为它要经 `Snapshot` 直接序列化给前端。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceState {
    pub id: u8,
    pub onoff: u8,
    pub level: u8,
    /// 0.1°C 定点。
    pub temp_x10: i16,
    pub err: u8,
}

impl DeviceState {
    pub fn new(id: u8) -> Self {
        DeviceState {
            id,
            onoff: 0,
            level: 0,
            temp_x10: 0,
            err: 0,
        }
    }

    /// 解析 7B 状态条目；长度不足返回 None。
    pub fn parse(entry: &[u8]) -> Option<Self> {
        if entry.len() < STATUS_ENTRY_SIZE {
            return None;
        }
        Some(DeviceState {
            id: entry[0],
            onoff: entry[1],
            level: entry[2],
            temp_x10: i16::from_le_bytes([entry[3], entry[4]]),
            err: entry[5],
        })
    }
}

/// UDP 链路（IO 层）。
pub struct UdpLink {
    socket: UdpSocket,
    peer: SocketAddr,
    queue: RetransmitQueue,
    seq: u16,
    last_rx: Option<Instant>,
    last_hb: Instant,
    link_up: bool,
}

impl UdpLink {
    pub fn new(peer: SocketAddr) -> Result<Self, String> {
        // 绑定 0.0.0.0:0（随机本地端口）：PC 与 ART-Pi 双向可达即可，
        // 不占用固定端口，避免与 EasyInput 固件的 :17333 或其他实例冲突。
        let socket =
            UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("UDP 绑定失败: {e}"))?;
        Ok(Self {
            socket,
            peer,
            queue: RetransmitQueue::new(),
            seq: 1,
            last_rx: None,
            last_hb: Instant::now(),
            link_up: false,
        })
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// 链路是否活着（3s 内收到过 ART-Pi 回帧）。
    pub fn is_up(&self) -> bool {
        self.link_up
    }

    /// 等待 Ack 的命令数。
    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }

    pub fn set_peer(&mut self, peer: SocketAddr) {
        if peer != self.peer {
            log_buffer::info(format!("链路目标变更 → {peer}"));
            self.peer = peer;
            // 目标变了，旧队列全部作废（Ack 永远不会来）。
            self.queue = RetransmitQueue::new();
        }
    }

    fn next_seq(&mut self) -> u16 {
        let s = self.seq;
        // 0 保留给"未初始化"语义，跳过。
        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.seq = 1;
        }
        s
    }

    /// 发送一条命令（首送即入重传队列）。
    pub fn send_command(&mut self, cmd: u8, payload: &[u8], label: &str) -> Result<(), String> {
        let seq = self.next_seq();
        let bytes = frame::encode(&Frame {
            frame_type: FT_CMD,
            cmd,
            seq,
            sid: 0,
            payload: payload.to_vec(),
        })?;
        self.socket
            .send_to(&bytes, self.peer)
            .map_err(|e| format!("UDP 发送失败: {e}"))?;
        self.queue
            .track(cmd, payload.to_vec(), 0, seq, label.to_string(), Instant::now());
        log_buffer::info(format!(
            "发送 {label}（cmd=0x{cmd:02X} seq={seq} payload={}）",
            hex(payload)
        ));
        Ok(())
    }

    /// 一次周期事务：收包（至多阻塞 `timeout`）→ 重传扫描 → 心跳 → 判活。
    pub fn pump(&mut self, timeout: Duration) -> Vec<LinkEvent> {
        let mut events = Vec::new();
        self.recv_once(&mut events, timeout);
        self.poll_retransmits(&mut events);
        self.heartbeat(&mut events);
        self.check_alive(&mut events);
        events
    }

    fn recv_once(&mut self, events: &mut Vec<LinkEvent>, timeout: Duration) {
        self.socket
            .set_read_timeout(Some(timeout))
            .expect("set_read_timeout 不应失败");
        let mut buf = [0u8; 256]; // SH_CTRL_RX_BUF_SIZE 留裕量
        match self.socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                self.last_rx = Some(Instant::now());
                match frame::decode(&buf[..n]) {
                    Ok(f) => self.handle_frame(&f, events),
                    Err(e) => events.push(LinkEvent::Log(format!(
                        "丢弃来自 {from} 的坏帧（{n}B）: {e}"
                    ))),
                }
            }
            // ⚠️ **Windows 上读超时的 ErrorKind 是 `TimedOut`，不是 `WouldBlock`**。
            // 2026-10-15 实测踩过：`set_read_timeout(50ms)` 到期后 `recv_from`
            // 返回 `WSAETIMEDOUT (10060)`，`ErrorKind::WouldBlock` 判据落空，
            // 于是走else 分支每 50ms 往事件队列塞一条错误日志——UI 日志面板
            // 每秒被刷掉 20 条「UDP 接收错误」，真故障信息全被埋掉。
            //
            // 两个 kind 都当作「本轮没等到包」，安静返回：读超时是 50ms 轮询的
            // **正常**结果，不是错误。
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => events.push(LinkEvent::Log(format!("UDP 接收错误: {e}"))),
        }
    }

    fn handle_frame(&mut self, f: &Frame, events: &mut Vec<LinkEvent>) {
        match f.frame_type {
            FT_ACK => {
                // 查询类 Ack 的载荷就是状态条目。
                if (f.cmd == CMD_QUERY_STATUS || f.cmd == CMD_QUERY_ALL) && !f.payload.is_empty() {
                    let states = parse_status_payload(&f.payload);
                    if !states.is_empty() {
                        events.push(LinkEvent::Status(states));
                    }
                }
                match self.queue.on_ack(f.seq) {
                    Some(p) => events.push(LinkEvent::Acked {
                        cmd: p.cmd,
                        label: p.label,
                    }),
                    None => {
                        // 心跳 Ack 不带跟踪；也可能是迟到的重传 Ack——都无需动作。
                    }
                }
            }
            FT_STATUS => {
                let states = parse_status_payload(&f.payload);
                if !states.is_empty() {
                    events.push(LinkEvent::Status(states));
                }
            }
            _ => events.push(LinkEvent::Log(format!(
                "忽略帧 type=0x{:02X} cmd=0x{:02X} seq={}",
                f.frame_type, f.cmd, f.seq
            ))),
        }
    }

    fn poll_retransmits(&mut self, events: &mut Vec<LinkEvent>) {
        // 借用拆分：poll 需要 &mut self.queue，而 seq 分配需要 &mut self.seq。
        // 两者是 self 的不同字段，可同时借；先取出队列的到期结果（ owned ），
        // 再动 socket 发送，避免"持着队列借用发 IO"的写法。
        let now = Instant::now();
        let queue = &mut self.queue;
        let seq_cell = &mut self.seq;
        let mut alloc_seq = || {
            let s = *seq_cell;
            *seq_cell = seq_cell.wrapping_add(1);
            if *seq_cell == 0 {
                *seq_cell = 1;
            }
            s
        };
        let due = queue.poll(now, &mut alloc_seq);
        drop(alloc_seq);
        for d in due {
            match d {
                Due::Retransmit(bytes) => {
                    if let Err(e) = self.socket.send_to(&bytes, self.peer) {
                        events.push(LinkEvent::Log(format!("重传发送失败: {e}")));
                    }
                }
                Due::Failed(p) => events.push(LinkEvent::Failed {
                    cmd: p.cmd,
                    label: p.label,
                }),
            }
        }
    }

    fn heartbeat(&mut self, events: &mut Vec<LinkEvent>) {
        if self.last_hb.elapsed() < Duration::from_millis(HEARTBEAT_INTERVAL_MS) {
            return;
        }
        self.last_hb = Instant::now();
        let seq = self.next_seq();
        if let Ok(bytes) = frame::encode(&Frame {
            frame_type: FT_CMD,
            cmd: CMD_HEARTBEAT,
            seq,
            sid: 0,
            payload: vec![],
        }) {
            // 心跳不跟踪重传：Ack 丢失本身即"链路可疑"的信号，
            // 由 check_alive 的 3s 判死统一处理，避免心跳塞满重传队列。
            if let Err(e) = self.socket.send_to(&bytes, self.peer) {
                events.push(LinkEvent::Log(format!("心跳发送失败: {e}")));
            }
        }
    }

    fn check_alive(&mut self, events: &mut Vec<LinkEvent>) {
        let alive = self
            .last_rx
            .map(|t| t.elapsed() < Duration::from_millis(LINK_DEAD_AFTER_MS))
            .unwrap_or(false);
        if alive && !self.link_up {
            self.link_up = true;
            events.push(LinkEvent::LinkUp);
        } else if !alive && self.link_up {
            self.link_up = false;
            events.push(LinkEvent::LinkDown);
        }
    }
}

/// 解析 Ack / 状态帧载荷里的连续 7B 设备条目。
pub fn parse_status_payload(payload: &[u8]) -> Vec<DeviceState> {
    payload
        .chunks_exact(STATUS_ENTRY_SIZE)
        .filter_map(DeviceState::parse)
        .collect()
}

/// 载荷 hex 摘要（日志用，超长截断）。
pub fn hex(bytes: &[u8]) -> String {
    let take = bytes.len().min(16);
    let mut s: Vec<String> = bytes[..take].iter().map(|b| format!("{b:02X}")).collect();
    if bytes.len() > take {
        s.push("…".into());
    }
    s.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Instant {
        Instant::now()
    }

    #[test]
    fn track_and_ack_removes_pending() {
        let mut q = RetransmitQueue::new();
        let t0 = base();
        q.track(CMD_LIGHT_ONOFF, vec![1, 1], 0, 100, "主灯开".into(), t0);
        assert_eq!(q.len(), 1);
        assert!(q.on_ack(100).is_some());
        assert!(q.is_empty());
        // 重复 Ack 不再命中（防重放语义与 ART-Pi 一致）。
        assert!(q.on_ack(100).is_none());
    }

    #[test]
    fn ack_with_foreign_seq_is_ignored() {
        let mut q = RetransmitQueue::new();
        q.track(CMD_LIGHT_ONOFF, vec![1, 1], 0, 100, "主灯开".into(), base());
        assert!(q.on_ack(999).is_none());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn no_retransmit_before_200ms() {
        let mut q = RetransmitQueue::new();
        let t0 = base();
        q.track(CMD_HEARTBEAT, vec![], 0, 1, "心跳".into(), t0);
        let mut seq = 200u16;
        let mut alloc = || {
            seq += 1;
            seq
        };
        assert!(q.poll(t0 + Duration::from_millis(199), &mut alloc).is_empty());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn retransmit_uses_fresh_seq() {
        // 这条测试钉死"最隐蔽的坑"：重传帧的 seq 必须与首送不同。
        let mut q = RetransmitQueue::new();
        let t0 = base();
        q.track(CMD_LIGHT_LEVEL, vec![1, 80], 0, 100, "主灯80%".into(), t0);
        let mut seq = 200u16;
        let mut alloc = || {
            seq += 1;
            seq
        };
        let due = q.poll(t0 + Duration::from_millis(200), &mut alloc);
        assert_eq!(due.len(), 1);
        match &due[0] {
            Due::Retransmit(bytes) => {
                let f = frame::decode(bytes).unwrap();
                assert_ne!(f.seq, 100, "重传必须换 seq，否则被 ART-Pi 去重窗口丢弃");
                assert_eq!(f.seq, 201);
                assert_eq!(f.cmd, CMD_LIGHT_LEVEL);
                assert_eq!(f.payload, vec![1, 80]);
            }
            Due::Failed(_) => panic!("200ms 时不应失败"),
        }
    }

    #[test]
    fn gives_up_after_max_retransmits() {
        let mut q = RetransmitQueue::new();
        let t0 = base();
        q.track(CMD_FAN_ONOFF, vec![3, 1], 0, 100, "风扇开".into(), t0);
        let mut seq = 200u16;
        let mut alloc = || {
            seq += 1;
            seq
        };
        let mut t = t0;
        let mut failed = 0;
        // 首送 + 3 次重传 = 4 帧；第 5 个 200ms 窗口到期即判失败。
        for _ in 0..4 {
            t += Duration::from_millis(200);
            for d in q.poll(t, &mut alloc) {
                if let Due::Failed(p) = d {
                    failed += 1;
                    assert_eq!(p.cmd, CMD_FAN_ONOFF);
                }
            }
        }
        t += Duration::from_millis(200);
        for d in q.poll(t, &mut alloc) {
            if let Due::Failed(p) = d {
                failed += 1;
                assert_eq!(p.cmd, CMD_FAN_ONOFF);
            }
        }
        assert_eq!(failed, 1, "重传次数用尽后应失败且仅一次");
        assert!(q.is_empty());
    }

    #[test]
    fn ack_after_retransmit_clears_queue() {
        let mut q = RetransmitQueue::new();
        let t0 = base();
        q.track(CMD_SCENE, vec![1], 0, 100, "回家".into(), t0);
        let mut seq = 200u16;
        let mut alloc = || {
            seq += 1;
            seq
        };
        let due = q.poll(t0 + Duration::from_millis(200), &mut alloc);
        let new_seq = match &due[0] {
            Due::Retransmit(bytes) => frame::decode(bytes).unwrap().seq,
            _ => panic!(),
        };
        // ART-Pi 对重传帧（新 seq）回 Ack。
        assert!(q.on_ack(new_seq).is_some());
        assert!(q.is_empty());
    }

    #[test]
    fn parse_status_entries() {
        let mut payload = Vec::new();
        // 24.0℃ = 240 = 0x00F0 → 小端 [0xF0, 0x00]。
        payload.extend_from_slice(&[1, 1, 80, 0xF0, 0x00, 0, 0]);
        // 17.6℃ = 176 = 0x00B0 → 小端 [0xB0, 0x00]。
        payload.extend_from_slice(&[4, 1, 0, 0xB0, 0x00, 0, 0]);
        let states = parse_status_payload(&payload);
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].id, 1);
        assert_eq!(states[0].temp_x10, 240);
        assert_eq!(states[1].temp_x10, 176);
        // 非整数倍尾部被忽略（容错，不 panic）。
        let states = parse_status_payload(&[1, 1, 80]);
        assert!(states.is_empty());
    }

    /// Windows 读超时是 `TimedOut`（WSAETIMEDOUT 10060）而不是 `WouldBlock`。
    ///
    /// 这不是"顺手补的"：判据写错时，50ms 轮询每轮都产出一条错误日志，
    /// UI 日志面板每秒被刷掉 20 条，真故障信息全被埋。
    #[test]
    fn read_timeout_is_not_treated_as_error() {
        // 一个没人监听的地址：构造链路后在50ms 内必然收不到包。
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut link = UdpLink::new(peer).expect("绑定本机随机端口应成功");

        let mut events = Vec::new();
        link.recv_once(&mut events, Duration::from_millis(50));

        // 关键：超时不得产生任何事件（尤其不得是 Log）。
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LinkEvent::Log(m) if m.contains("接收错误"))),
            "读超时被当成错误上报了：{events:?}"
        );
        assert!(
            events.is_empty(),
            "读超时应安静返回，实际产出：{events:?}"
        );
    }

    /// 超时安静返回的前提：别把它误改成"吞掉一切"。
    #[test]
    fn recv_once_ignores_unrelated_timeout_but_frame_still_parses() {
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut link = UdpLink::new(peer).expect("绑定应成功");

        // 对端不存在 ⇒ 可能收到 ICMP 不可达，也可能是纯超时；
        // 两种都不该panic，且 last_rx 不应被更新。
        for _ in 0..3 {
            let mut events = Vec::new();
            link.recv_once(&mut events, Duration::from_millis(20));
        }
        assert!(link.last_rx.is_none(), "无对端时不得更新 last_rx");
    }
}
