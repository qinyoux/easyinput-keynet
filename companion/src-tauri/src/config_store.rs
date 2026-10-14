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
pub const SCENE_MODE_EXPAND: &str = "expand"; // Companion 展开成设备命令（MVP 默认）
pub const SCENE_MODE_REMOTE: &str = "remote"; // 只发 0x47，等 ART-Pi M4 场景展开落地

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

/// 完整配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeMap {
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

fn default_scene_mode() -> String {
    SCENE_MODE_EXPAND.to_string()
}

impl Default for HomeMap {
    fn default() -> Self {
        let mut bindings = Vec::new();
        // KEY1 → 回家，KEY2 → 影院（演示最常用的两个场景）。
        bindings.push(Binding {
            uuid: crate::constants::scene_uuid(1).unwrap().to_string(),
            action: Action::Scene { scene: 1 },
        });
        bindings.push(Binding {
            uuid: crate::constants::scene_uuid(4).unwrap().to_string(),
            action: Action::Scene { scene: 4 },
        });
        // KEY3–KEY8 → 6 台设备直控。
        for dev in 1..=6u8 {
            bindings.push(Binding {
                uuid: crate::constants::device_toggle_uuid(dev).unwrap().to_string(),
                action: Action::DeviceToggle { device: dev },
            });
        }
        // 编码器三向。
        bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_LEFT.to_string(),
            action: Action::EncoderLeft,
        });
        bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_RIGHT.to_string(),
            action: Action::EncoderRight,
        });
        bindings.push(Binding {
            uuid: crate::constants::UUID_ENCODER_PRESS.to_string(),
            action: Action::EncoderPress,
        });
        HomeMap {
            artpi_ip: DEFAULT_ART_PI_IP.to_string(),
            artpi_port: default_port(),
            scene_mode: default_scene_mode(),
            bindings,
            scenes: default_scenes(),
        }
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

/// 兜底清洗：端口 0、空 IP、非法 scene_mode、空场景表都拉回合法值。
/// 宁可在加载时修掉，也不要让坏配置在运行期炸链路。
fn sanitize(map: &mut HomeMap) {    if map.artpi_ip.trim().is_empty() {
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
        assert_eq!(map.scene_mode, SCENE_MODE_EXPAND);
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
