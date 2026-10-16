//! V2.3 统一帧编解码 + seq 去重窗口（纯逻辑，无 IO，可单测）。
//!
//! 与 ART-Pi `frame_codec.c` 逐字节对齐：
//! - 线上格式大端（seq）、CRC 小端存储、CRC 覆盖前 9+len 字节；
//! - decode 是唯一字节序转换点，业务层永远面向 `Frame` 编程；
//! - `SeqWindow` 复刻 ART-Pi 16 槽位图语义（回绕差分判定：领先前移 /
//!   出窗旧流重置 / 同 seq 与窗内已见即重放）。
//!
//! ⚠️ PC 侧**不**用 SeqWindow 去重（那是接收方的事）；这里实现它是为了
//! 单测锁死 ART-Pi 的行为假设——尤其"重传必须换 seq"这条（见
//! `RetransmitQueue` 的测试）。

use crate::constants::*;
use crate::crc16::crc16_ccitt;

/// 解码后的逻辑帧（与线上字节序解耦）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// 帧类型（FT_CMD / FT_STATUS / FT_ACK / FT_CFG_REQ / FT_CFG_DATA）。
    pub frame_type: u8,
    pub cmd: u8,
    pub seq: u16,
    pub sid: u8,
    pub payload: Vec<u8>,
}

/// 逻辑帧 → 线上字节流（追加 CRC）。
pub fn encode(frame: &Frame) -> Result<Vec<u8>, String> {
    if frame.payload.len() > FRAME_PAYLOAD_MAX {
        return Err(format!(
            "载荷越界: {}（上限 {FRAME_PAYLOAD_MAX}）",
            frame.payload.len()
        ));
    }
    let need = FRAME_HEADER_SIZE + frame.payload.len() + 2;
    let mut buf = vec![0u8; need];
    buf[0] = FRAME_HEAD1;
    buf[1] = FRAME_HEAD2;
    buf[2] = FRAME_VERSION;
    buf[3] = frame.frame_type;
    buf[4] = frame.cmd;
    buf[5] = frame.payload.len() as u8;
    buf[6] = (frame.seq >> 8) as u8; // 大端
    buf[7] = (frame.seq & 0xFF) as u8;
    buf[8] = frame.sid;
    buf[FRAME_HEADER_SIZE..FRAME_HEADER_SIZE + frame.payload.len()]
        .copy_from_slice(&frame.payload);
    let crc = crc16_ccitt(&buf[..FRAME_HEADER_SIZE + frame.payload.len()]);
    buf[need - 2] = (crc & 0xFF) as u8; // 小端存储
    buf[need - 1] = (crc >> 8) as u8;
    Ok(buf)
}

/// 线上字节流 → 逻辑帧。校验帧头/版本/长度/CRC，任一不符即拒绝。
///
/// 注意与 ART-Pi 的错误码差异：C 侧返回 `SH_ERR_CHECKSUM` / `SH_ERR_RANGE`，
/// PC 侧统一用 `Err(String)` 带可读原因（日志要能一眼看出是哪一关没过）。
pub fn decode(buf: &[u8]) -> Result<Frame, String> {
    if buf.len() < FRAME_HEADER_SIZE + 2 {
        return Err(format!("帧过短: {}（不足帧头+CRC）", buf.len()));
    }
    if buf[0] != FRAME_HEAD1 || buf[1] != FRAME_HEAD2 {
        return Err(format!("魔数不符: {:02X} {:02X}", buf[0], buf[1]));
    }
    if buf[2] != FRAME_VERSION {
        return Err(format!("版本不符: 0x{:02X}", buf[2]));
    }
    let len = buf[5] as usize;
    let need = FRAME_HEADER_SIZE + len + 2;
    if buf.len() < need {
        return Err(format!("声明长度与实际不符: len={len}, 实际 {}", buf.len()));
    }
    let crc_calc = crc16_ccitt(&buf[..FRAME_HEADER_SIZE + len]);
    let crc_rx = u16::from_le_bytes([buf[need - 2], buf[need - 1]]);
    if crc_calc != crc_rx {
        return Err(format!(
            "CRC 不符: 收 {crc_rx:04X} / 算 {crc_calc:04X}"
        ));
    }
    Ok(Frame {
        frame_type: buf[3],
        cmd: buf[4],
        seq: u16::from_be_bytes([buf[6], buf[7]]),
        sid: buf[8],
        payload: buf[FRAME_HEADER_SIZE..FRAME_HEADER_SIZE + len].to_vec(),
    })
}

/// seq 去重滑动窗口——**逐位复刻** ART-Pi `sh_frame_seq_fresh` 的槽位图语义。
///
/// 判定用无符号 16 位回绕差分（RFC 1982 思路）：
/// - `diff == 0`：与最新帧同 seq，重放，拒；
/// - `diff ∈ [1, WINDOW)`：落后 1..15，查位图，置位即重放/乱序重传，拒；
/// - `diff ∈ [WINDOW, 0x8000)`：落后 16..32767，窗口外旧流（对端重启），重置放行；
/// - `diff >= 0x8000`：seq 领先（正常递增流），窗口前移，放行。
///
/// 演进记录（2026-10-16）：旧版把 `diff >= WINDOW` 一律当新流重置，递增流每帧
/// 重置、位图永远只剩 1 位，last-1 重放拦不住（实测前丢过包）。现按差分符号
/// 分流。存在的意义是单测：PC 侧靠 `RetransmitQueue` 换 seq 来绕过这个窗口，
/// 测试必须先证明"同 seq 重传会被吃"这件事在这里成立。
#[derive(Debug, Default)]
pub struct SeqWindow {
    last: u16,
    mask: u32,
    inited: bool,
}

impl SeqWindow {
    pub fn fresh(&mut self, seq: u16) -> bool {
        if !self.inited {
            self.inited = true;
            self.last = seq;
            self.mask = 1;
            return true;
        }
        let diff = self.last.wrapping_sub(seq);
        if diff == 0 {
            return false; // 与最新帧同 seq：重放
        }
        if diff < FRAME_SEQ_WINDOW {
            let bit = 1u32 << diff;
            if self.mask & bit != 0 {
                return false; // 窗口内已见：重放/乱序重传
            }
            self.mask |= bit;
            return true;
        }
        if diff < 0x8000 {
            // 落后 16..32767：窗口外旧流（对端重启/换流），重置窗口。
            self.last = seq;
            self.mask = 1;
            return true;
        }
        // diff >= 0x8000：seq 领先 last（正常递增流），窗口前移。
        let advance = 0x10000u32 - diff as u32; // 1..32767
        if advance >= FRAME_SEQ_WINDOW as u32 {
            self.mask = 1; // 整窗跳出，只留最新一槽
        } else {
            self.mask = ((self.mask << advance) | 1) & 0xFFFF;
        }
        self.last = seq;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame {
            frame_type: FT_CMD,
            cmd: CMD_LIGHT_ONOFF,
            seq: 0x0102,
            sid: 7,
            payload: vec![DEV_ID_MAIN_LIGHT, 1],
        }
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let f = sample();
        let bytes = encode(&f).unwrap();
        assert_eq!(bytes.len(), FRAME_HEADER_SIZE + 2 + 2);
        assert_eq!(&bytes[0..2], &[FRAME_HEAD1, FRAME_HEAD2]);
        assert_eq!(bytes[2], FRAME_VERSION);
        let back = decode(&bytes).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn seq_is_big_endian_on_wire() {
        let bytes = encode(&sample()).unwrap();
        assert_eq!(bytes[6], 0x01);
        assert_eq!(bytes[7], 0x02);
    }

    #[test]
    fn crc_is_little_endian_on_wire() {
        let f = Frame {
            frame_type: FT_CMD,
            cmd: CMD_HEARTBEAT,
            seq: 1,
            sid: 0,
            payload: vec![],
        };
        let bytes = encode(&f).unwrap();
        let crc = crc16_ccitt(&bytes[..FRAME_HEADER_SIZE]);
        assert_eq!(bytes[FRAME_HEADER_SIZE], (crc & 0xFF) as u8);
        assert_eq!(bytes[FRAME_HEADER_SIZE + 1], (crc >> 8) as u8);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = encode(&sample()).unwrap();
        bytes[0] = 0x00;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_bad_version() {
        let mut bytes = encode(&sample()).unwrap();
        bytes[2] = 0x03;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_bad_crc() {
        let mut bytes = encode(&sample()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_truncated() {
        let bytes = encode(&sample()).unwrap();
        assert!(decode(&bytes[..FRAME_HEADER_SIZE + 1]).is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn decode_rejects_len_mismatch() {
        // 声明 len=5 但实际只给 11 字节。
        let bytes = encode(&sample()).unwrap();
        let mut short = bytes[..FRAME_HEADER_SIZE + 2].to_vec();
        short[5] = 5;
        assert!(decode(&short).is_err());
    }

    #[test]
    fn encode_rejects_oversize_payload() {
        let f = Frame {
            frame_type: FT_CMD,
            cmd: CMD_CFG_WRITE,
            seq: 0,
            sid: 0,
            payload: vec![0u8; FRAME_PAYLOAD_MAX + 1],
        };
        assert!(encode(&f).is_err());
    }

    #[test]
    fn multi_device_status_payload_roundtrips() {
        // 0x48 全设备查询 Ack：6 台 × 7B = 42B 载荷。
        let mut payload = Vec::new();
        for id in 1..=6u8 {
            payload.extend_from_slice(&[id, 1, 80, 0x00, 0x0A, 0, 0]); // 24.0°C
        }
        let f = Frame {
            frame_type: FT_ACK,
            cmd: CMD_QUERY_ALL,
            seq: 0xFFFF,
            sid: 0,
            payload,
        };
        let back = decode(&encode(&f).unwrap()).unwrap();
        assert_eq!(back.payload.len(), 6 * STATUS_ENTRY_SIZE);
    }

    #[test]
    fn seq_window_matches_art_pi_semantics() {
        let mut w = SeqWindow::default();
        // 首次 seq 视为新流。
        assert!(w.fresh(100));
        // 同 seq 重放：丢弃。
        assert!(!w.fresh(100));
        // 递增流：窗口前移并累积（2026-10-16 修复后的语义）。
        assert!(w.fresh(101));
        assert!(!w.fresh(101));
        // 100 此刻已入窗（不再是"跳变重置"），重放必被吃——
        // 这正是修复的核心：递增流下 last-1/last-2 重放拦得住。
        assert!(!w.fresh(100));
        // 推进到窗口边缘（16）之外：出窗旧流，重置放行。
        assert!(w.fresh(120));
        // 重置后旧的 seq 又变"新"。
        assert!(w.fresh(100));
    }

    #[test]
    fn seq_window_increasing_stream_blocks_replays() {
        // 回归镜像 ART-Pi host_test/test_frame_codec.c 用例 3：
        // 修复前（diff>=WINDOW 一律重置）这里 101/100 会被漏放。
        let mut w = SeqWindow::default();
        assert!(w.fresh(100));
        assert!(w.fresh(101));
        assert!(w.fresh(102));
        assert!(!w.fresh(101), "last-1 重放必被吃");
        assert!(!w.fresh(100), "last-2 重放必被吃");
        assert!(!w.fresh(102), "last 自身重放必被吃");
        assert!(w.fresh(103), "正常递增继续放行");
        assert!(!w.fresh(100), "窗口前移后仍在 16 槽内");
    }

    #[test]
    fn seq_window_wraparound_65535_to_0() {
        let mut w = SeqWindow::default();
        assert!(w.fresh(0xFFFE));
        assert!(w.fresh(0xFFFF));
        assert!(w.fresh(0x0000), "回绕点必须无缝");
        assert!(w.fresh(0x0001));
        assert!(!w.fresh(0xFFFF), "回绕后重放照拦");
        assert!(!w.fresh(0x0000));
        assert!(w.fresh(0x0002));
    }

    #[test]
    fn seq_window_peer_restart_tolerated() {
        let mut w = SeqWindow::default();
        assert!(w.fresh(500));
        assert!(w.fresh(2), "对端重启回落到小值：出窗旧流，重置放行（不锁死链路）");
        assert!(w.fresh(3));
        assert!(!w.fresh(2), "新流的重放照样拦");
    }

    #[test]
    fn same_seq_retransmit_would_be_dropped() {
        // 这条测试钉死"最隐蔽的坑"：如果 PC 重传不换 seq，
        // ART-Pi 的 SeqWindow 会把它当重放吃掉。
        let mut w = SeqWindow::default();
        assert!(w.fresh(42));
        assert!(!w.fresh(42), "同 seq 重传必被 ART-Pi 丢弃");
        assert!(w.fresh(43), "换新 seq 才能送达");
    }
}
