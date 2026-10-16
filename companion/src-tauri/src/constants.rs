//! 集中常量。三个真源，互不覆盖：
//!
//! 1. **HID 侧**（EasyInput 开发板固件 `easy-input-maker`）：`host_action_protocol.h`
//!    / `usb_hid.cpp` —— 0x11 输入报告、kind 0x05、36 字节 UUID；
//! 2. **V2.3 帧侧**（ART-Pi `applications/inc/sh_frame.h` + `sh_config.h`）：
//!    0x5A 0xA5 + CRC16-CCITT + seq 去重窗口 16；
//! 3. ** Companion 自身约定**：保留 UUID 段、默认场景表、重传参数。

/// EasyInput 开发板 USB 标识（固件 `usb_hid.cpp`）。
pub const VID: u16 = 0x303A;
pub const PID: u16 = 0x1006;

/// 回执/命令通道（开发板 → APP）。
pub const REPORT_ID_APP_COMMAND: u8 = 0x11;

/// 0x11 kind：Host Action（按键绑定 `host_action:<uuid>` 时回发）。
pub const KIND_HOST_ACTION: u8 = 0x05;

/// ERROR_IO_PENDING 的 HRESULT（HRESULT_FROM_WIN32(997)，异步 IO 正常进行中）。
pub const HRESULT_ERROR_IO_PENDING: u32 = 0x8007_03E5;

/// 0x11 kind 0x05 报文：`[0]=0x11 [1]=kind [2..6]=4B header [6..42]=36B UUID`。
pub const HOST_ACTION_V1_HEADER_LEN: usize = 4;
pub const HOST_ACTION_V1_UUID_LEN: usize = 36;

// ============================================================================
// V2.3 统一帧（与 ART-Pi `sh_frame.h` / `sh_config.h` 逐字段对齐）
// ============================================================================

/// 帧头魔数。
pub const FRAME_HEAD1: u8 = 0x5A;
pub const FRAME_HEAD2: u8 = 0xA5;
/// 帧版本（ART-Pi `SH_FRAME_VERSION`）。
pub const FRAME_VERSION: u8 = 0x02;
/// 帧头定长：`0x5A 0xA5 ver type cmd len seq(2) sid`。
pub const FRAME_HEADER_SIZE: usize = 9;
/// 载荷上限（保证整帧 ≤250B 不触发 IP 分片）。
pub const FRAME_PAYLOAD_MAX: usize = 239;
/// 整帧上限。
pub const FRAME_MAX: usize = 250;
/// ART-Pi seq 去重滑动窗口大小（`SH_FRAME_SEQ_WINDOW`）。
///
/// ⚠️ **这是本工程最隐蔽的坑**：ART-Pi 用 16 槽位图记录最近 16 个 seq，
/// 窗口内重复 seq 的帧**直接丢弃**。因此 PC 侧重传**必须换新 seq**，
/// 否则重传帧被当成重放吃掉，命令永久丢失（表现为"发了没反应"）。
pub const FRAME_SEQ_WINDOW: u16 = 16;

/// 帧类型（帧偏移 3）。
pub const FT_CMD: u8 = 1;
pub const FT_STATUS: u8 = 2;
pub const FT_ACK: u8 = 3;
pub const FT_CFG_REQ: u8 = 4;
pub const FT_CFG_DATA: u8 = 5;

/// 命令码（帧偏移 4）。沿用段 + 家居段，与 ART-Pi `sh_frame.h` 一致。
pub const CMD_QUERY_STATUS: u8 = 0x20; // 载荷: device_id → Ack 回 7B 状态
pub const CMD_HEARTBEAT: u8 = 0x30; // 载荷: 无 → 回空 Ack
pub const CMD_LIGHT_ONOFF: u8 = 0x40; // 载荷: device_id, onoff
pub const CMD_LIGHT_LEVEL: u8 = 0x41; // 载荷: device_id, level(0-100)
pub const CMD_FAN_ONOFF: u8 = 0x42; // 载荷: device_id, onoff
pub const CMD_FAN_LEVEL: u8 = 0x43; // 载荷: device_id, level(0-100)
pub const CMD_THERMO_TARGET: u8 = 0x44; // 载荷: zone, temp_x10_lo, temp_x10_hi
pub const CMD_THERMO_MODE: u8 = 0x45; // 载荷: zone, mode(0关/1冷/2热/3自动)
pub const CMD_SOCKET_ONOFF: u8 = 0x46; // 载荷: device_id, onoff
pub const CMD_SCENE: u8 = 0x47; // 载荷: scene_id(1回家/2离家/3睡眠/4影院)
pub const CMD_QUERY_ALL: u8 = 0x48; // 载荷: 无 → Ack 回 N×7B 聚合状态
pub const CMD_CFG_WRITE: u8 = 0x50; // 配置分块写入（type 4/5 机制）

/// 状态帧单设备条目大小：`device_id, onoff, level, temp_x10_lo, temp_x10_hi, err, rsv`。
pub const STATUS_ENTRY_SIZE: usize = 7;

/// 设备编号（ART-Pi `sh_config.h`，TDD 2.4 执行端规划）。
pub const DEV_ID_MAIN_LIGHT: u8 = 1; // 主灯：继电器总开关 + PWM 调光
pub const DEV_ID_LIGHT_STRIP: u8 = 2; // 灯带：PWM 调光
pub const DEV_ID_FAN: u8 = 3; // 风扇：PWM 调速
pub const DEV_ID_THERMO: u8 = 4; // 温控执行：继电器通断 + SHT30 闭环
pub const DEV_ID_SOCKET1: u8 = 5; // 插座 1：GPIO 继电器
pub const DEV_ID_SOCKET2: u8 = 6; // 插座 2：GPIO 继电器（预留）

/// 温控模式（ART-Pi `sh_types.h`）。
pub const THERMO_MODE_OFF: u8 = 0;
pub const THERMO_MODE_COOL: u8 = 1;
pub const THERMO_MODE_HEAT: u8 = 2;
pub const THERMO_MODE_AUTO: u8 = 3;

/// ART-Pi 控制通道端口（`SH_CTRL_UDP_PORT`）。
pub const ART_PI_PORT: u16 = 17335;

/// 同一轮内多条命令之间的发送间隔（ms）。
///
/// **ART-Pi 真机实测约束**（2026-10-15）：一个场景展开出的 2–6 条命令若无
/// 间隔连发，板子会随机丢包——8 连发仅 1/8 收到 Ack，且 3 次重传全丢
/// ⇒ 场景执行不全。根因在板子侧（`ctrl_server.c` 每包 `rt_kprintf` 阻塞
/// 收包线程 + AP6212 接收缓冲），而路线 D 的前提是固件零改动，所以只能
/// 由 PC 侧让出节奏。
///
/// 为什么重传兜不住：重传 200ms 后才触发，板子丢包属接收缓冲溢出型，
/// 重发同样进不去（实测 3 次全丢）。从源头降速才有效。
///
/// 取值依据：实测 0ms→1/8、50ms→7/8、150ms→31/31（连跑 3 次零丢失）。
/// 真人按键天然 >200ms 间隔，故这是"对齐真实使用节奏"，不是迁就 bug。
pub const CMD_GAP_MS: u64 = 150;

// ============================================================================
// Companion 自身约定
// ============================================================================

/// 关键命令重传间隔（ART-Pi `SH_ACK_TIMEOUT_MS`：发送端承担）。
pub const RETRANSMIT_AFTER_MS: u64 = 200;
/// 关键命令最大重传次数（首送 + 3 次重传 = 最多 4 帧）。
pub const RETRANSMIT_MAX: u8 = 3;

/// 心跳——**自适应双档**（2026-10-15 钦友要求：串口留给调试，别被心跳刷屏）。
///
/// 板子端 `ctrl_server.c` 每收一帧都 `rt_kprintf` 打日志，固定 800ms 心跳 =
/// 串口每分钟 75 条噪音，真调试信息全被淹没。改为两档：
///
/// - **探测档**（[`HEARTBEAT_PROBE_MS`]）：未连通（含启动）或链路可疑
///   （有命令重传耗尽）时使用。300ms 快速探到板子，比旧 800ms 首连更快。
/// - **保活档**（[`HEARTBEAT_IDLE_MS`]）：已连通且无异常时使用。5s 一跳，
///   串口噪音从 75 帧/分降到 12 帧/分。
///
/// ⚠️ 三个值必须联动，否则会出现「判死 → 高频探测 → 复活 → 放缓 → 又判死」
/// 的振荡，比固定 800ms 更吵：
/// [`HEARTBEAT_IDLE_MS`] + 余量 必须 > [`LINK_DEAD_AFTER_MS`]——保活档下
/// 单个心跳 Ack 丢失后，PC 要等到下一跳（5s 后）才有回帧，3s 旧阈值会在
/// 每个保活间隙误判死链。8s = 5s + 一个重传周期余量。
/// 板子侧「1s 无帧判 PC 失联」只影响板子的在线指示，不影响命令执行
/// （命令收帧独立处理，真机 8 场景 31 命令全 Ack 已验证）。
pub const HEARTBEAT_PROBE_MS: u64 = 300;
pub const HEARTBEAT_IDLE_MS: u64 = 5000;

/// 链路判死：超过该时长未收到任何 ART-Pi 回帧（Ack/状态）即置链路 Down。
pub const LINK_DEAD_AFTER_MS: u64 = 8000;

/// 保留 UUID 段：家居场景（8 个）。
///
/// 与 easyop 的保留 UUID 段（`00000000-0000-4000-8000-0000000000XX`，01–10）
/// **故意错开**：固件对 `host_action:<uuid>` 是白名单制但语义中性——它只原样回发，
/// 语义全在主机端。两个上位机若共用同一块板子，UUID 撞车 = 一个键被两处认领。
/// 本段用 `...0000000001XX`（场景）与 `...0000000002XX`（设备直控）。
pub fn scene_uuid(scene: u8) -> Option<&'static str> {
    match scene {
        1 => Some("00000000-0000-4000-8000-000000000101"),
        2 => Some("00000000-0000-4000-8000-000000000102"),
        3 => Some("00000000-0000-4000-8000-000000000103"),
        4 => Some("00000000-0000-4000-8000-000000000104"),
        5 => Some("00000000-0000-4000-8000-000000000105"),
        6 => Some("00000000-0000-4000-8000-000000000106"),
        7 => Some("00000000-0000-4000-8000-000000000107"),
        8 => Some("00000000-0000-4000-8000-000000000108"),
        _ => None,
    }
}

/// 保留 UUID 段：设备直控开关（6 台设备，KEY3–KEY8 默认绑定用）。
pub fn device_toggle_uuid(device: u8) -> Option<&'static str> {
    match device {
        1 => Some("00000000-0000-4000-8000-000000000201"),
        2 => Some("00000000-0000-4000-8000-000000000202"),
        3 => Some("00000000-0000-4000-8000-000000000203"),
        4 => Some("00000000-0000-4000-8000-000000000204"),
        5 => Some("00000000-0000-4000-8000-000000000205"),
        6 => Some("00000000-0000-4000-8000-000000000206"),
        _ => None,
    }
}

/// 保留 UUID：编码器左旋（主灯亮度 −10）。
pub const UUID_ENCODER_LEFT: &str = "00000000-0000-4000-8000-000000000301";
/// 保留 UUID：编码器右旋（主灯亮度 +10）。
pub const UUID_ENCODER_RIGHT: &str = "00000000-0000-4000-8000-000000000302";
/// 保留 UUID：编码器按下（主灯开关翻转）。
pub const UUID_ENCODER_PRESS: &str = "00000000-0000-4000-8000-000000000303";

/// 设备编号 → 中文显示名（前端与日志共用）。
pub fn device_name(id: u8) -> &'static str {
    match id {
        DEV_ID_MAIN_LIGHT => "主灯",
        DEV_ID_LIGHT_STRIP => "灯带",
        DEV_ID_FAN => "风扇",
        DEV_ID_THERMO => "温控",
        DEV_ID_SOCKET1 => "插座1",
        DEV_ID_SOCKET2 => "插座2",
        _ => "未知设备",
    }
}

/// 温控模式 → 中文显示名。
pub fn thermo_mode_name(mode: u8) -> &'static str {
    match mode {
        THERMO_MODE_OFF => "关",
        THERMO_MODE_COOL => "制冷",
        THERMO_MODE_HEAT => "制热",
        THERMO_MODE_AUTO => "自动",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_uuid_segments_do_not_collide_with_easyop() {
        // easyop 保留段 01–10（000000000001..000000000010）一个都不许撞。
        for s in 1..=8u8 {
            let u = scene_uuid(s).unwrap();
            assert_eq!(u.len(), HOST_ACTION_V1_UUID_LEN);
            assert!(!u.ends_with("000000000001"), "场景 UUID 撞 easyop 保留段");
            for tail in 1..=10u8 {
                let easyop = format!("00000000-0000-4000-8000-{:012}", tail);
                assert_ne!(u, easyop);
            }
        }
        for d in 1..=6u8 {
            let u = device_toggle_uuid(d).unwrap();
            for tail in 1..=10u8 {
                let easyop = format!("00000000-0000-4000-8000-{:012}", tail);
                assert_ne!(u, easyop);
            }
        }
        assert_ne!(UUID_ENCODER_LEFT, UUID_ENCODER_RIGHT);
        assert_ne!(UUID_ENCODER_LEFT, UUID_ENCODER_PRESS);
        assert_ne!(UUID_ENCODER_RIGHT, UUID_ENCODER_PRESS);
    }

    #[test]
    fn scene_uuid_covers_exactly_eight_scenes() {
        for s in 1..=8u8 {
            assert!(scene_uuid(s).is_some());
        }
        assert!(scene_uuid(0).is_none());
        assert!(scene_uuid(9).is_none());
        // 8 个场景 UUID 互不相同。
        let mut all: Vec<&str> = (1..=8u8).map(|s| scene_uuid(s).unwrap()).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 8);
    }
}
