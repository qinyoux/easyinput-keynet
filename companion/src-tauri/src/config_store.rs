//! `home_map.json` 配置：ART-Pi 端点、UUID 绑定表、场景表。
//!
//! 存放位置：`{app_config_dir}/home_map.json`（Tauri `path().app_config_dir()`，
//! Windows 上是 `%APPDATA%/com.easyinput.companion/`）。首次运行用内置默认表，
//! 之后一切修改走 `save_map` 落盘——**配置即状态，重启不丢**。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::home::scene::{default_scenes, SceneDef};

/// 默认 ART-Pi 地址（可在界面改）。ART-Pi 经 wifi_manager DHCP 拿地址，
/// 首次联调时以 FinSH `ifconfig` 输出为准。
pub const DEFAULT_ART_PI_IP: &str = "192.168.1.10";

/// 场景展开模式。
///
/// 2026-10-16 起默认 `remote`：ART-Pi 的 `on_scene` 已落地真实现，只发 1 个
/// 0x47 由板子本地展开，触摸场景栏高亮才会跟着 EasyInput 同步。
pub const SCENE_MODE_EXPAND: &str = "expand"; // Companion 展开成设备命令（老固件退路）
pub const SCENE_MODE_REMOTE: &str = "remote"; // 只发 0x47，ART-Pi 本地展开（默认）

/// 一条 UUID → 动作绑定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub uuid: String,
    #[serde(flatten)]
    pub action: Action,
}

/// 绑定动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// 激活场景（Companion 展开或远程 0x47，取决于 scene_mode）。
    Scene { scene: u8 },
    /// 单设备开关翻转。
    DeviceToggle { device: u8 },
    /// 编码器左旋：主灯亮度 −10。
    EncoderLeft,
    /// 编码器右旋：主灯亮度 +10。
    EncoderRight,
    /// 编码器按下：主灯开关翻转。
    EncoderPress,
}

impl Action {
    /// 界面显示用的一句话。
    pub fn label(&self) -> String {
        match self {
            Action::Scene { scene } => format!("场景 {scene}"),
            Action::DeviceToggle { device } => {
                format!("{} 开关", crate::constants::device_name(*device))
            }
            Action::EncoderLeft => "编码器左旋（亮度−10）".into(),
            Action::EncoderRight => "编码器右旋（亮度+10）".into(),
            Action::EncoderPress => "编码器按下（主灯翻转）".into(),
        }
    }
}

/// 当前配置结构版本。用于一次性迁移老配置（见 `migrate`）。
///
/// v1：初版，无显式版本字段（serde 缺省为 0）。
/// v2：默认 `scene_mode` 由 `expand` 改为 `remote`（ART-Pi 已能本地展开）。
pub const CONFIG_VERSION: u32 = 2;

/// 完整配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeMap {
    /// 配置结构版本。`#[serde(default)]` 让老文件缺这个字段时读成 0 → 触发迁移。
    #[serde(default)]
    pub config_version: u32,
    pub artpi_ip: String,
    #[serde(default = "default_port")]
    pub artpi_port: u16,
    #[serde(default = "default_scene_mode")]
    pub scene_mode: String,
    pub bindings: Vec<Binding>,
    pub scenes: Vec<SceneDef>,
}

fn default_port() -> u16 {
    crate::constants::ART_PI_PORT
}

/// 默认场景模式：**remote**（只发 0x47，由 ART-Pi 本地展开）。
///
/// 2026-10-16 从 expand 改为 remote。ART-Pi 的 `on_scene` 已于 2026-10-16
/// 从 `TODO(M4)` 桩落地为真实现（8 场景 30 命令，三端 parity 锁定），
/// 原先「ART-Pi 不会展开所以必须 PC 侧展开」的前提已不成立。
///
/// 为什么不留 expand 作默认（它仍然可用，作为老固件/排障的退路）：
/// expand 模式下 Companion 把场景拆成 4–6 条设备命令逐条发，ART-Pi
/// **从未收到 0x47**，`sh_scene_execute()` / `sh_ui_set_active_scene()`
/// 一次都不会被调用——结果是灯和上位机都同步了，唯独触摸屏场景栏
/// 高亮不动（2026-10-16 真机实测）。remote 模式一个包就够，高亮由
/// `sh_scene_execute` 单点收口，触摸 / EasyInput / FinSH 三路自动一致；
/// 顺带把 4–6 个包降到 1 个，`CMD_GAP_MS=150ms` 节流约束也不再需要。
fn default_scene_mode() -> String {
    SCENE_MODE_REMOTE.to_string()
}

impl Default for HomeMap {
    fn default() -> Self {
        let mut map = HomeMap {
            config_version: CONFIG_VERSION,
            artpi_ip: DEFAULT_ART_PI_IP.to_string(),
            artpi_port: default_port(),
            scene_mode: default_scene_mode(),
            bindings: Vec::new(),
            scenes: default_scenes(),
        };
        // KEY1 → 回家，KEY2 → 影院（演示最常用的两个场景）。
        map.bindings.push(Binding {
            uuid: crate::constants::scene_uuid(1).unwrap().to_string(),
            action: Action::Scene { scene: 1 },
        });
        map.bindings.push(Binding {
            uuid: crate::constants::scene_uuid(4).unwrap().to_string(),
            action: Action::Scene { scene: 4 },
        });
        // KEY3–KEY8 → 6 台设备直控。
        for dev in 1..=6u8 {
            map.bindings.push(Binding {
                uuid: crate::constants::device_toggle_uuid(dev).unwrap().to_string(),
                action: Action::DeviceToggle { device: dev },
            });
        }
        // 编码器三向。
        map.bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_LEFT.to_string(),
            action: Action::EncoderLeft,
        });
        map.bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_RIGHT.to_string(),
            action: Action::EncoderRight,
        });
        map.bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_PRESS.to_string(),
            action: Action::EncoderPress,
        });
        map
    }
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("home_map.json")
}

/// 加载配置：磁盘优先，损坏/缺失回退默认（并在日志说明）。
pub fn load(dir: &Path) -> HomeMap {
    let path = config_path(dir);
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<HomeMap>(&text) {
            Ok(mut map) => {
                sanitize(&mut map);
                crate::log_buffer::info(format!("已加载配置 {}", path.display()));
                map
            }
            Err(e) => {
                crate::log_buffer::warn(format!(
                    "配置 {} 解析失败（{e}），本次使用默认表；修正后可用「保存配置」覆盖",
                    path.display()
                ));
                HomeMap::default()
            }
        },
        Err(_) => {
            crate::log_buffer::info("未找到配置文件，使用内置默认映射表");
            HomeMap::default()
        }
    }
}

/// 保存配置（自动建目录）。
pub fn save(dir: &Path, map: &HomeMap) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    let path = config_path(dir);
    let text = serde_json::to_string_pretty(map).map_err(|e| format!("序列化失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写配置失败: {e}"))?;
    crate::log_buffer::info(format!("配置已保存 {}", path.display()));
    Ok(path)
}

/// 一次性配置迁移：老配置（config_version < 2）里写死了 `scene_mode:"expand"`，
/// 那会让 ART-Pi 收不到 0x47、触摸屏场景栏高亮永远不同步（2026-10-16 真机实测）。
/// 这里把它拉回 `remote`，**并保留 expand 的显式选择权**：只有版本号落后的
/// 老配置才被改；用户在新版里手动选 expand，保存后版本号已是 2，不会被再改。
fn migrate(map: &mut HomeMap) {
    if map.config_version >= CONFIG_VERSION {
        return;
    }
    if map.scene_mode == SCENE_MODE_EXPAND {
        map.scene_mode = SCENE_MODE_REMOTE.to_string();
        crate::log_buffer::info(
            "配置已升级到 v2：场景模式 expand → remote（ART-Pi 本地展开，触摸高亮同步）"
                .to_string(),
        );
    }
    map.config_version = CONFIG_VERSION;
}

/// 兜底清洗：端口 0、空 IP、非法 scene_mode、空场景表都拉回合法值。
/// 宁可在加载时修掉，也不要让坏配置在运行期炸链路。
fn sanitize(map: &mut HomeMap) {
    migrate(map);
    if map.artpi_ip.trim().is_empty() {
        map.artpi_ip = DEFAULT_ART_PI_IP.to_string();
    }
    if map.artpi_port == 0 {
        map.artpi_port = default_port();
    }
    if map.scene_mode != SCENE_MODE_EXPAND && map.scene_mode != SCENE_MODE_REMOTE {
        map.scene_mode = default_scene_mode();
    }
    if map.scenes.is_empty() {
        map.scenes = default_scenes();
    }
    map.bindings
        .retain(|b| crate::hid::host_action::is_canonical_uuid(&b.uuid));
}

/// 对外暴露的清洗入口（`update_map` 命令用：前端传来的配置也要过同一道关）。
pub fn sanitize_pub(map: &mut HomeMap) {
    sanitize(map);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_bindings_are_unique_and_canonical() {
        let map = HomeMap::default();
        let mut uuids: Vec<&str> = map.bindings.iter().map(|b| b.uuid.as_str()).collect();
        let before = uuids.len();
        uuids.sort_unstable();
        uuids.dedup();
        assert_eq!(uuids.len(), before, "UUID 绑定必须唯一");
        for b in &map.bindings {
            assert!(crate::hid::host_action::is_canonical_uuid(&b.uuid));
        }
    }

    #[test]
    fn default_map_covers_key1_key2_and_all_devices() {
        let map = HomeMap::default();
        assert!(map
            .bindings
            .iter()
            .any(|b| matches!(b.action, Action::Scene { scene: 1 })));
        assert!(map
            .bindings
            .iter()
            .any(|b| matches!(b.action, Action::Scene { scene: 4 })));
        for dev in 1..=6u8 {
            assert!(map
                .bindings
                .iter()
                .any(|b| matches!(b.action, Action::DeviceToggle { device } if device == dev)));
        }
    }

    #[test]
    fn serde_roundtrip() {
        let map = HomeMap::default();
        let text = serde_json::to_string_pretty(&map).unwrap();
        let back: HomeMap = serde_json::from_str(&text).unwrap();
        assert_eq!(map, back);
    }

    /// **老配置迁移（v1 → v2）**：真机缺陷的第二个入口。
    ///
    /// 只改 `default_scene_mode()` 是不够的——用户磁盘上的 home_map.json
    /// 里明明白白写着 `"scene_mode": "expand"`，加载时会被原样保留，
    /// 高亮不同步照旧。这条用例钉住"老配置必须被拉回 remote"。
    #[test]
    fn migrate_v1_expand_config_to_remote() {
        // 复刻一份 v1 老配置：无 config_version 字段 + 显式 expand。
        let legacy = r#"{
            "artpi_ip": "10.10.21.29",
            "artpi_port": 17335,
            "scene_mode": "expand",
            "bindings": [],
            "scenes": []
        }"#;
        let mut map: HomeMap = serde_json::from_str(legacy).unwrap();
        assert_eq!(map.config_version, 0, "老配置没有版本字段，应读成 0");
        assert_eq!(map.scene_mode, SCENE_MODE_EXPAND);

        sanitize(&mut map);
        assert_eq!(
            map.scene_mode, SCENE_MODE_REMOTE,
            "v1 老配置的 expand 必须被迁移成 remote"
        );
        assert_eq!(map.config_version, CONFIG_VERSION);
    }

    /// 迁移只做一次：新版里用户**主动**选 expand，不该被再改回去。
    /// 否则「expand 作为老固件退路」的承诺就是空的。
    #[test]
    fn migrate_respects_explicit_expand_on_v2() {
        let mut map = HomeMap::default();
        map.scene_mode = SCENE_MODE_EXPAND.to_string();
        assert_eq!(map.config_version, CONFIG_VERSION);
        sanitize(&mut map);
        assert_eq!(
            map.scene_mode, SCENE_MODE_EXPAND,
            "已是 v2 的显式 expand 选择必须被尊重（老固件退路）"
        );
    }

    /// 缺 config_version 且已是 remote 的老配置：迁移只补版本号，不动模式。
    #[test]
    fn migrate_does_not_touch_remote_mode() {
        let legacy = r#"{
            "artpi_ip": "10.10.21.29",
            "artpi_port": 17335,
            "scene_mode": "remote",
            "bindings": [],
            "scenes": []
        }"#;
        let mut map: HomeMap = serde_json::from_str(legacy).unwrap();
        sanitize(&mut map);
        assert_eq!(map.scene_mode, SCENE_MODE_REMOTE);
        assert_eq!(map.config_version, CONFIG_VERSION);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("companion-test-home-map");
        let _ = fs::remove_dir_all(&dir);
        let mut map = HomeMap::default();
        map.artpi_ip = "10.0.0.5".into();
        map.scene_mode = SCENE_MODE_REMOTE.into();
        save(&dir, &map).unwrap();
        let back = load(&dir);
        assert_eq!(back.artpi_ip, "10.0.0.5");
        assert_eq!(back.scene_mode, SCENE_MODE_REMOTE);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_repairs_broken_map() {
        let mut map = HomeMap::default();
        map.artpi_ip = "  ".into();
        map.artpi_port = 0;
        map.scene_mode = "bogus".into();
        map.bindings.push(Binding {
            uuid: "not-a-uuid".into(),
            action: Action::Scene { scene: 1 },
        });
        sanitize(&mut map);
        assert_eq!(map.artpi_ip, DEFAULT_ART_PI_IP);
        assert_eq!(map.artpi_port, crate::constants::ART_PI_PORT);
        // 非法 scene_mode 回落到**当前**默认（不写死字符串）——2026-10-16
        // 默认由 expand 改 remote，这条断言跟着改过一次，写死会再次腐烂。
        assert_eq!(map.scene_mode, default_scene_mode());
        assert_eq!(map.scene_mode, SCENE_MODE_REMOTE);
        assert!(!map.bindings.iter().any(|b| b.uuid == "not-a-uuid"));
    }

    #[test]
    fn load_missing_dir_falls_back_to_default() {
        let dir = std::env::temp_dir().join("companion-test-nonexistent-dir-xyz");
        let _ = fs::remove_dir_all(&dir);
        let map = load(&dir);
        assert_eq!(map, HomeMap::default());
    }
}
