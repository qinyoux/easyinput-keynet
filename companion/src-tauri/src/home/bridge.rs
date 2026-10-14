//! 桥接层：HID 上报的 UUID → 家居动作 → V2.3 命令 → UDP 发送通道。
//!
//! 数据流：
//! ```text
//! EasyInput 板 --USB HID 0x11--> 监听线程 --UUID--> bridge::dispatch
//!   --> 查 HomeMap.bindings --> Action --> SceneManager（本地状态）
//!   --> OutCmd 通道 --> UDP 线程 --> ART-Pi :17335
//! ```
//!
//! 为什么中间要过一条通道而不是在监听线程里直接发 UDP：监听线程的节拍是
//! "读到 → 交出 → 立刻回去读"，`send_to` 虽快，但重传队列、Ack 等待、
//! 目标切换都归 UDP 线程所有——两个关注点分开，任一边卡住不拖另一边。

use std::sync::mpsc::Sender;

use crate::config_store::{Action, HomeMap, SCENE_MODE_REMOTE};
use crate::constants::*;
use crate::home::scene::SceneManager;
use crate::log_buffer;

/// 发往 UDP 线程的命令。
pub enum OutCmd {
    /// 发送一条 V2.3 命令。
    Send {
        cmd: u8,
        payload: Vec<u8>,
        label: String,
    },
    /// 切换链路目标（IP 变更）。
    SetPeer(String),
    /// 立即查询全部设备状态。
    QueryAll,
}

/// UUID → 绑定动作。
pub fn resolve_action(uuid: &str, map: &HomeMap) -> Option<Action> {
    map.bindings
        .iter()
        .find(|b| b.uuid == uuid)
        .map(|b| b.action.clone())
}

/// 执行一个 UUID 触发的家居动作。
///
/// 返回人类可读结果（调用方记日志）。错误（如场景未定义）也返回字符串，
/// **不静默**——"按键没反应"必须有日志可查。
pub fn dispatch(
    uuid: &str,
    map: &HomeMap,
    mgr: &mut SceneManager,
    tx: &Sender<OutCmd>,
) -> Result<String, String> {
    let action = match resolve_action(uuid, map) {
        Some(a) => a,
        None => {
            crate::log_buffer::warn(format!(
                "UUID {uuid} 未绑定任何动作（在「配置」页绑定，或在 easyop 里改键位）"
            ));
            return Err(format!("UUID {uuid} 未绑定"));
        }
    };
    match action {
        Action::Scene { scene } => {
            if map.scene_mode == SCENE_MODE_REMOTE {
                // 远程模式：只发 0x47，由 ART-Pi 展开（M4 落地后启用）。
                let name = mgr
                    .find(scene)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| format!("场景{scene}"));
                mgr.set_active(scene);
                send(tx, CMD_SCENE, &[scene], format!("场景「{name}」（远程展开）"));
                Ok(format!("已发送场景「{name}」（0x47，ART-Pi 本地展开）"))
            } else {
                // 展开模式：Companion 逐条发设备命令。
                let cmds = mgr.activate(scene)?;
                let name = mgr.find(scene).map(|s| s.name.clone()).unwrap_or_default();
                for (cmd, payload, label) in cmds {
                    send(tx, cmd, &payload, label);
                }
                Ok(format!("已激活场景「{name}」"))
            }
        }
        Action::DeviceToggle { device } => {
            let (cmd, payload, label) = mgr.toggle_device(device)?;
            send(tx, cmd, &payload, label.clone());
            Ok(label)
        }
        Action::EncoderLeft => {
            let (cmd, payload, label) = mgr.adjust_main_light(-10)?;
            send(tx, cmd, &payload, label.clone());
            Ok(label)
        }
        Action::EncoderRight => {
            let (cmd, payload, label) = mgr.adjust_main_light(10)?;
            send(tx, cmd, &payload, label.clone());
            Ok(label)
        }
        Action::EncoderPress => {
            let (cmd, payload, label) = mgr.toggle_device(DEV_ID_MAIN_LIGHT)?;
            send(tx, cmd, &payload, label.clone());
            Ok(label)
        }
    }
}

/// 调试注入器：直接发一条命令（不经场景机，本地状态不更新——权威状态
/// 由随后的 Ack/状态帧回传）。
pub fn inject(tx: &Sender<OutCmd>, cmd: u8, payload: &[u8]) -> Result<String, String> {
    let label = describe_cmd(cmd, payload);
    send(tx, cmd, payload, label.clone());
    Ok(format!("已注入 {label}"))
}

fn send(tx: &Sender<OutCmd>, cmd: u8, payload: &[u8], label: String) {
    if let Err(e) = tx.send(OutCmd::Send {
        cmd,
        payload: payload.to_vec(),
        label,
    }) {
        log_buffer::err(format!("命令通道已断开: {e}"));
    }
}

/// 命令码 → 中文描述（日志与调试台共用）。
pub fn describe_cmd(cmd: u8, payload: &[u8]) -> String {
    let dev = |i: usize| payload.get(i).copied().unwrap_or(0);
    match cmd {
        CMD_LIGHT_ONOFF => format!("{}{}", device_name(dev(0)), if dev(1) == 1 { "开" } else { "关" }),
        CMD_LIGHT_LEVEL => format!("{}亮度{}%", device_name(dev(0)), dev(1)),
        CMD_FAN_ONOFF => format!("风扇{}", if dev(1) == 1 { "开" } else { "关" }),
        CMD_FAN_LEVEL => format!("风扇挡位{}%", dev(1)),
        CMD_THERMO_TARGET => {
            let t = i16::from_le_bytes([dev(1), dev(2)]);
            format!("温控目标{}.{}℃", t / 10, t % 10)
        }
        CMD_THERMO_MODE => format!("温控模式{}", thermo_mode_name(dev(1))),
        CMD_SOCKET_ONOFF => format!("{}{}", device_name(dev(0)), if dev(1) == 1 { "开" } else { "关" }),
        CMD_SCENE => format!("场景{}", dev(0)),
        CMD_QUERY_STATUS => format!("查询设备{}", device_name(dev(0))),
        CMD_QUERY_ALL => "查询全部设备".into(),
        CMD_HEARTBEAT => "心跳".into(),
        CMD_CFG_WRITE => "配置写入".into(),
        _ => format!("cmd=0x{cmd:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::scene::SceneManager;

    fn setup() -> (HomeMap, SceneManager, Sender<OutCmd>, std::sync::mpsc::Receiver<OutCmd>) {
        let map = HomeMap::default();
        let mgr = SceneManager::new(map.scenes.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        (map, mgr, tx, rx)
    }

    #[test]
    fn resolve_default_bindings() {
        let map = HomeMap::default();
        assert_eq!(
            resolve_action(crate::constants::scene_uuid(1).unwrap(), &map),
            Some(Action::Scene { scene: 1 })
        );
        assert_eq!(
            resolve_action(crate::constants::device_toggle_uuid(3).unwrap(), &map),
            Some(Action::DeviceToggle { device: 3 })
        );
        assert_eq!(
            resolve_action(crate::constants::UUID_ENCODER_LEFT, &map),
            Some(Action::EncoderLeft)
        );
        assert!(resolve_action("00000000-0000-4000-8000-000000000999", &map).is_none());
    }

    #[test]
    fn dispatch_scene_expand_sends_device_commands() {
        let (map, mut mgr, tx, rx) = setup();
        let msg = dispatch(crate::constants::scene_uuid(4).unwrap(), &map, &mut mgr, &tx).unwrap();
        assert!(msg.contains("影院"));
        // 影院 = 主灯30% + 插座1开 = 2 条命令。
        assert_eq!(rx.try_iter().count(), 2);
        assert_eq!(mgr.active(), Some(4));
    }

    #[test]
    fn dispatch_scene_remote_sends_single_0x47() {
        let (mut map, mut mgr, tx, rx) = setup();
        map.scene_mode = SCENE_MODE_REMOTE.to_string();
        dispatch(crate::constants::scene_uuid(1).unwrap(), &map, &mut mgr, &tx).unwrap();
        let cmds: Vec<OutCmd> = rx.try_iter().collect();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            OutCmd::Send { cmd, payload, .. } => {
                assert_eq!(*cmd, CMD_SCENE);
                assert_eq!(payload, &vec![1]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn dispatch_device_toggle_updates_state() {
        let (map, mut mgr, tx, rx) = setup();
        dispatch(
            crate::constants::device_toggle_uuid(1).unwrap(),
            &map,
            &mut mgr,
            &tx,
        )
        .unwrap();
        assert_eq!(rx.try_iter().count(), 1);
        let main = mgr.states().into_iter().find(|s| s.id == 1).unwrap();
        assert_eq!(main.onoff, 1);
    }

    #[test]
    fn dispatch_encoder_adjusts_level() {
        let (map, mut mgr, tx, rx) = setup();
        dispatch(crate::constants::UUID_ENCODER_RIGHT, &map, &mut mgr, &tx).unwrap();
        assert_eq!(rx.try_iter().count(), 1);
        let main = mgr.states().into_iter().find(|s| s.id == 1).unwrap();
        assert_eq!(main.level, 10);
        dispatch(crate::constants::UUID_ENCODER_LEFT, &map, &mut mgr, &tx).unwrap();
        let main = mgr.states().into_iter().find(|s| s.id == 1).unwrap();
        assert_eq!(main.level, 0);
    }

    #[test]
    fn dispatch_unbound_uuid_is_error_not_silent() {
        let (map, mut mgr, tx, _rx) = setup();
        let r = dispatch("00000000-0000-4000-8000-000000000999", &map, &mut mgr, &tx);
        assert!(r.is_err());
    }

    #[test]
    fn describe_cmd_covers_wire_commands() {
        assert_eq!(describe_cmd(CMD_LIGHT_ONOFF, &[1, 1]), "主灯开");
        assert_eq!(describe_cmd(CMD_LIGHT_LEVEL, &[1, 80]), "主灯亮度80%");
        assert_eq!(describe_cmd(CMD_THERMO_TARGET, &[4, 0xF0, 0x00]), "温控目标24.0℃");
        assert_eq!(describe_cmd(CMD_THERMO_MODE, &[4, 3]), "温控模式自动");
        assert_eq!(describe_cmd(CMD_SCENE, &[4]), "场景4");
    }
}
