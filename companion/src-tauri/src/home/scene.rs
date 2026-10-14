//! 家居场景状态机：8 场景定义、UUID→动作解析、设备状态跟踪。
//!
//! # 为什么场景在 PC 侧展开成设备命令
//!
//! TDD 原设计是「场景 0x47 在 ART-Pi 本地展开」，但 ART-Pi `cmd_dispatch.c`
//! 的 `on_scene` 目前是 `TODO(M4)` 桩（收了帧什么都不做）。方案 D 的 MVP
//! 必须**今天就能演示**，因此 Companion 把场景展开成 0x40–0x46 设备命令逐条
//! 发送——这些命令 ART-Pi 侧已完整实现（继电器/SHT30 联调目标）。
//!
//! 配置项 `scene_mode` 保留两条路：
//! - `"expand"`（默认，MVP）：Companion 展开，ART-Pi 零依赖；
//! - `"remote"`：只发 0x47，等 ART-Pi M4 落地后切换。
//! 两条路都安全：设备命令幂等，不会双执行。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::log_buffer;

/// 场景内的一条设备命令（配置态，可序列化进 home_map.json）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SceneCmd {
    LightOnOff { dev: u8, on: bool },
    LightLevel { dev: u8, level: u8 },
    FanOnOff { dev: u8, on: bool },
    FanLevel { dev: u8, level: u8 },
    ThermoTarget { zone: u8, temp_x10: i16 },
    ThermoMode { zone: u8, mode: u8 },
    SocketOnOff { dev: u8, on: bool },
}

impl SceneCmd {
    /// 转成 V2.3 线上命令（cmd + payload）。
    pub fn to_wire(&self) -> (u8, Vec<u8>) {
        match *self {
            SceneCmd::LightOnOff { dev, on } => (CMD_LIGHT_ONOFF, vec![dev, on as u8]),
            SceneCmd::LightLevel { dev, level } => (CMD_LIGHT_LEVEL, vec![dev, level]),
            SceneCmd::FanOnOff { dev, on } => (CMD_FAN_ONOFF, vec![dev, on as u8]),
            SceneCmd::FanLevel { dev, level } => (CMD_FAN_LEVEL, vec![dev, level]),
            SceneCmd::ThermoTarget { zone, temp_x10 } => (
                CMD_THERMO_TARGET,
                vec![zone, (temp_x10 & 0xFF) as u8, ((temp_x10 >> 8) & 0xFF) as u8],
            ),
            SceneCmd::ThermoMode { zone, mode } => (CMD_THERMO_MODE, vec![zone, mode]),
            SceneCmd::SocketOnOff { dev, on } => (CMD_SOCKET_ONOFF, vec![dev, on as u8]),
        }
    }

    /// 日志用的一句话描述。
    pub fn label(&self) -> String {
        match *self {
            SceneCmd::LightOnOff { dev, on } => {
                format!("{}{}", device_name(dev), if on { "开" } else { "关" })
            }
            SceneCmd::LightLevel { dev, level } => format!("{}亮度{}%", device_name(dev), level),
            SceneCmd::FanOnOff { dev, on } => {
                format!("{}{}", device_name(dev), if on { "开" } else { "关" })
            }
            SceneCmd::FanLevel { dev, level } => format!("{}挡位{}%", device_name(dev), level),
            SceneCmd::ThermoTarget { zone, temp_x10 } => {
                format!("温控{zone}目标{}.{}℃", temp_x10 / 10, temp_x10 % 10)
            }
            SceneCmd::ThermoMode { zone, mode } => {
                format!("温控{zone}模式{}", thermo_mode_name(mode))
            }
            SceneCmd::SocketOnOff { dev, on } => {
                format!("{}{}", device_name(dev), if on { "开" } else { "关" })
            }
        }
    }

    /// 乐观本地状态：这条命令会把哪台设备改成什么样（可视化用）。
    fn apply_to(&self, states: &mut BTreeMap<u8, DeviceState>) {
        match *self {
            SceneCmd::LightOnOff { dev, on } => entry(states, dev).onoff = on as u8,
            SceneCmd::LightLevel { dev, level } => entry(states, dev).level = level,
            SceneCmd::FanOnOff { dev, on } => entry(states, dev).onoff = on as u8,
            SceneCmd::FanLevel { dev, level } => entry(states, dev).level = level,
            SceneCmd::ThermoTarget { .. } => {}
            SceneCmd::ThermoMode { .. } => {}
            SceneCmd::SocketOnOff { dev, on } => entry(states, dev).onoff = on as u8,
        }
    }
}

/// 设备运行状态。
///
/// **唯一类型真源在 `v23::udp_client::DeviceState`**（链路回传的权威状态与
/// 本地乐观状态必须同型，否则每次转换都是一处潜伏的字段错位）；这里再导出
/// 只是让场景机的调用方少写一层路径。
pub use crate::v23::udp_client::DeviceState;

fn entry(states: &mut BTreeMap<u8, DeviceState>, id: u8) -> &mut DeviceState {
    states.entry(id).or_insert_with(|| DeviceState::new(id))
}

/// 一个场景：编号、名称、展开命令序列。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDef {
    pub id: u8,
    pub name: String,
    pub commands: Vec<SceneCmd>,
}

/// 场景状态机（设备状态 + 当前场景）。
pub struct SceneManager {
    scenes: Vec<SceneDef>,
    active: Option<u8>,
    states: BTreeMap<u8, DeviceState>,
}

impl SceneManager {
    pub fn new(scenes: Vec<SceneDef>) -> Self {
        let mut mgr = SceneManager {
            scenes,
            active: None,
            states: BTreeMap::new(),
        };
        // 预置 6 台设备，前端首屏即有完整卡片（未知设备 id 由状态帧补入）。
        for id in 1..=6u8 {
            mgr.states.insert(id, DeviceState::new(id));
        }
        mgr
    }

    pub fn scenes(&self) -> &[SceneDef] {
        &self.scenes
    }

    pub fn active(&self) -> Option<u8> {
        self.active
    }

    /// 远程场景模式下由桥接层置位（不展开设备命令，本地状态等回传）。
    pub fn set_active(&mut self, id: u8) {
        if self.find(id).is_some() {
            self.active = Some(id);
        }
    }

    /// 替换场景表（配置保存后同步；保留设备状态与当前激活位）。
    pub fn replace_scenes(&mut self, scenes: Vec<SceneDef>) {
        self.scenes = scenes;
        if let Some(a) = self.active {
            if self.find(a).is_none() {
                self.active = None;
            }
        }
    }

    pub fn states(&self) -> Vec<DeviceState> {
        self.states.values().cloned().collect()
    }

    pub fn find(&self, id: u8) -> Option<&SceneDef> {
        self.scenes.iter().find(|s| s.id == id)
    }

    /// 激活场景：返回要发送的 (cmd, payload, label) 序列，并更新本地状态。
    pub fn activate(&mut self, id: u8) -> Result<Vec<(u8, Vec<u8>, String)>, String> {
        let scene = self
            .find(id)
            .ok_or_else(|| format!("场景 {id} 未定义（合法 1..=8）"))?
            .clone();
        let mut out = Vec::with_capacity(scene.commands.len());
        for cmd in &scene.commands {
            let (c, p) = cmd.to_wire();
            out.push((c, p, cmd.label()));
            cmd.apply_to(&mut self.states);
        }
        self.active = Some(id);
        log_buffer::info(format!("激活场景「{}」（{} 条命令）", scene.name, out.len()));
        Ok(out)
    }

    /// 用链路回传的状态覆盖本地视图（权威来源）。
    pub fn apply_status(&mut self, states: &[DeviceState]) {
        for s in states {
            entry(&mut self.states, s.id).clone_from(s);
        }
    }

    /// 单设备开关翻转（编码器按下 / KEY 直控用），返回命令序列。
    pub fn toggle_device(&mut self, id: u8) -> Result<(u8, Vec<u8>, String), String> {
        if !(1..=6).contains(&id) {
            return Err(format!("设备 {id} 不存在（合法 1..=6）"));
        }
        let cur = self
            .states
            .get(&id)
            .cloned()
            .unwrap_or_else(|| DeviceState::new(id));
        let on = cur.onoff == 0;
        let cmd = if matches!(id, DEV_ID_FAN) {
            SceneCmd::FanOnOff { dev: id, on }
        } else if matches!(id, DEV_ID_SOCKET1 | DEV_ID_SOCKET2) {
            SceneCmd::SocketOnOff { dev: id, on }
        } else {
            SceneCmd::LightOnOff { dev: id, on }
        };
        cmd.apply_to(&mut self.states);
        let (c, p) = cmd.to_wire();
        Ok((c, p, cmd.label()))
    }

    /// 主灯亮度相对调节（编码器左/右旋），返回命令序列。
    pub fn adjust_main_light(&mut self, delta: i32) -> Result<(u8, Vec<u8>, String), String> {
        let cur = self
            .states
            .get(&DEV_ID_MAIN_LIGHT)
            .cloned()
            .unwrap_or_else(|| DeviceState::new(DEV_ID_MAIN_LIGHT));
        let next = (cur.level as i32 + delta).clamp(0, 100) as u8;
        let cmd = SceneCmd::LightLevel {
            dev: DEV_ID_MAIN_LIGHT,
            level: next,
        };
        cmd.apply_to(&mut self.states);
        let (c, p) = cmd.to_wire();
        Ok((c, p, cmd.label()))
    }
}

/// 默认场景表（与 TDD 4.3 注释中的编译期桩对齐）。
pub fn default_scenes() -> Vec<SceneDef> {
    vec![
        SceneDef {
            id: 1,
            name: "回家".into(),
            commands: vec![
                SceneCmd::LightOnOff { dev: DEV_ID_MAIN_LIGHT, on: true },
                SceneCmd::LightLevel { dev: DEV_ID_MAIN_LIGHT, level: 80 },
                SceneCmd::ThermoTarget { zone: DEV_ID_THERMO, temp_x10: 240 },
                SceneCmd::ThermoMode { zone: DEV_ID_THERMO, mode: THERMO_MODE_AUTO },
            ],
        },
        SceneDef {
            id: 2,
            name: "离家".into(),
            commands: vec![
                SceneCmd::LightOnOff { dev: DEV_ID_MAIN_LIGHT, on: false },
                SceneCmd::LightOnOff { dev: DEV_ID_LIGHT_STRIP, on: false },
                SceneCmd::FanOnOff { dev: DEV_ID_FAN, on: false },
                SceneCmd::SocketOnOff { dev: DEV_ID_SOCKET1, on: false },
                SceneCmd::SocketOnOff { dev: DEV_ID_SOCKET2, on: false },
                SceneCmd::ThermoMode { zone: DEV_ID_THERMO, mode: THERMO_MODE_OFF },
            ],
        },
        SceneDef {
            id: 3,
            name: "睡眠".into(),
            commands: vec![
                SceneCmd::LightOnOff { dev: DEV_ID_MAIN_LIGHT, on: false },
                SceneCmd::LightLevel { dev: DEV_ID_MAIN_LIGHT, level: 0 },
                SceneCmd::ThermoTarget { zone: DEV_ID_THERMO, temp_x10: 260 },
            ],
        },
        SceneDef {
            id: 4,
            name: "影院".into(),
            commands: vec![
                SceneCmd::LightLevel { dev: DEV_ID_MAIN_LIGHT, level: 30 },
                SceneCmd::SocketOnOff { dev: DEV_ID_SOCKET1, on: true },
            ],
        },
        SceneDef {
            id: 5,
            name: "阅读".into(),
            commands: vec![
                SceneCmd::LightLevel { dev: DEV_ID_MAIN_LIGHT, level: 100 },
                SceneCmd::LightLevel { dev: DEV_ID_LIGHT_STRIP, level: 50 },
            ],
        },
        SceneDef {
            id: 6,
            name: "全开".into(),
            commands: vec![
                SceneCmd::LightOnOff { dev: DEV_ID_MAIN_LIGHT, on: true },
                SceneCmd::LightLevel { dev: DEV_ID_MAIN_LIGHT, level: 100 },
                SceneCmd::LightOnOff { dev: DEV_ID_LIGHT_STRIP, on: true },
                SceneCmd::LightLevel { dev: DEV_ID_LIGHT_STRIP, level: 100 },
            ],
        },
        SceneDef {
            id: 7,
            name: "全关".into(),
            commands: vec![
                SceneCmd::LightOnOff { dev: DEV_ID_MAIN_LIGHT, on: false },
                SceneCmd::LightOnOff { dev: DEV_ID_LIGHT_STRIP, on: false },
                SceneCmd::FanOnOff { dev: DEV_ID_FAN, on: false },
                SceneCmd::SocketOnOff { dev: DEV_ID_SOCKET1, on: false },
                SceneCmd::SocketOnOff { dev: DEV_ID_SOCKET2, on: false },
            ],
        },
        SceneDef {
            id: 8,
            name: "预冷".into(),
            commands: vec![
                SceneCmd::ThermoTarget { zone: DEV_ID_THERMO, temp_x10: 220 },
                SceneCmd::ThermoMode { zone: DEV_ID_THERMO, mode: THERMO_MODE_COOL },
                SceneCmd::FanOnOff { dev: DEV_ID_FAN, on: true },
                SceneCmd::FanLevel { dev: DEV_ID_FAN, level: 60 },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> SceneManager {
        SceneManager::new(default_scenes())
    }

    #[test]
    fn scene_cmd_wire_encoding_matches_art_pi_payload_layout() {
        // 0x44 温控目标：zone, temp_x10_lo, temp_x10_hi（小端）。
        let (cmd, p) = SceneCmd::ThermoTarget { zone: 4, temp_x10: 245 }.to_wire();
        assert_eq!(cmd, CMD_THERMO_TARGET);
        assert_eq!(p, vec![4, (245 & 0xFF) as u8, ((245 >> 8) & 0xFF) as u8]);
        // 0x40 灯开关：device_id, onoff。
        let (cmd, p) = SceneCmd::LightOnOff { dev: 1, on: true }.to_wire();
        assert_eq!(cmd, CMD_LIGHT_ONOFF);
        assert_eq!(p, vec![1, 1]);
        // 0x45 温控模式：zone, mode。
        let (cmd, p) = SceneCmd::ThermoMode { zone: 4, mode: THERMO_MODE_AUTO }.to_wire();
        assert_eq!(cmd, CMD_THERMO_MODE);
        assert_eq!(p, vec![4, 3]);
    }

    #[test]
    fn activate_scene_expands_and_updates_local_state() {
        let mut m = mgr();
        let cmds = m.activate(4).unwrap(); // 影院：主灯30% + 插座1开
        assert_eq!(cmds.len(), 2);
        assert_eq!(m.active(), Some(4));
        let main = m.states().into_iter().find(|s| s.id == 1).unwrap();
        assert_eq!(main.level, 30);
        let sock = m.states().into_iter().find(|s| s.id == 5).unwrap();
        assert_eq!(sock.onoff, 1);
    }

    #[test]
    fn activate_unknown_scene_is_error() {
        let mut m = mgr();
        assert!(m.activate(9).is_err());
        assert!(m.activate(0).is_err());
    }

    #[test]
    fn toggle_device_flips_state() {
        let mut m = mgr();
        let (cmd, p, _) = m.toggle_device(3).unwrap(); // 风扇：关 → 开
        assert_eq!(cmd, CMD_FAN_ONOFF);
        assert_eq!(p, vec![3, 1]);
        let (_, p2, _) = m.toggle_device(3).unwrap(); // 开 → 关
        assert_eq!(p2, vec![3, 0]);
        // 主灯与插座走各自命令码。
        let (cmd, _, _) = m.toggle_device(1).unwrap();
        assert_eq!(cmd, CMD_LIGHT_ONOFF);
        let (cmd, _, _) = m.toggle_device(5).unwrap();
        assert_eq!(cmd, CMD_SOCKET_ONOFF);
        assert!(m.toggle_device(7).is_err());
    }

    #[test]
    fn adjust_main_light_clamps_0_100() {
        let mut m = mgr();
        let (_, p, _) = m.adjust_main_light(-10).unwrap();
        assert_eq!(p[1], 0, "下限截断到 0");
        let (_, p, _) = m.adjust_main_light(35).unwrap();
        assert_eq!(p[1], 35);
        let (_, p, _) = m.adjust_main_light(80).unwrap();
        assert_eq!(p[1], 100, "上限截断到 100");
    }

    #[test]
    fn status_overrides_local_view() {
        use crate::v23::udp_client::DeviceState as Ds;
        let mut m = mgr();
        m.apply_status(&[Ds { id: 1, onoff: 1, level: 42, temp_x10: 0, err: 0 }]);
        let main = m.states().into_iter().find(|s| s.id == 1).unwrap();
        assert_eq!(main.level, 42, "链路回传状态是权威视图");
        // 未知设备 id 由状态帧补入。
        m.apply_status(&[Ds { id: 9, onoff: 1, level: 0, temp_x10: 0, err: 0 }]);
        assert!(m.states().iter().any(|s| s.id == 9));
    }

    #[test]
    fn default_scenes_cover_eight_and_are_unique() {
        let scenes = default_scenes();
        assert_eq!(scenes.len(), 8);
        for (i, s) in scenes.iter().enumerate() {
            assert_eq!(s.id as usize, i + 1);
            assert!(!s.name.is_empty());
            assert!(!s.commands.is_empty());
        }
    }

    #[test]
    fn home_scene_away_turns_everything_off() {
        let mut m = mgr();
        m.activate(6).unwrap(); // 全开
        m.activate(2).unwrap(); // 离家
        for s in m.states() {
            if s.id == 4 {
                continue; // 温控无开关位，走模式命令
            }
            assert_eq!(s.onoff, 0, "离家后设备 {} 应关闭", s.id);
        }
    }
}
