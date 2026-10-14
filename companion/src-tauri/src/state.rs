//! 共享状态与前端快照。
//!
//! 三把锁，**固定加锁顺序**（map → mgr → link），任何代码路径都不得逆序：
//! 逆序加锁是死锁的充分条件，而这里三个锁的持有时间都是微秒级，
//! 只要顺序一致就不可能互相等。

use std::sync::Mutex;

use serde::Serialize;

use crate::config_store::{Binding, HomeMap};
use crate::home::scene::{DeviceState, SceneDef, SceneManager};

/// 链路信息（前端链路指示灯）。
#[derive(Clone, Debug, Serialize)]
pub struct LinkInfo {
    pub up: bool,
    pub peer: String,
    /// 等待 Ack 的命令数（重传队列长度）。
    pub pending: usize,
}

/// 场景视图（只含前端要显示的字段）。
#[derive(Clone, Debug, Serialize)]
pub struct SceneView {
    pub id: u8,
    pub name: String,
    pub command_count: usize,
    pub active: bool,
}

/// 绑定视图（配置页复制 UUID 用）。
#[derive(Clone, Debug, Serialize)]
pub struct BindingView {
    pub uuid: String,
    pub action: String,
}

/// 整机快照（前端轮询/事件双驱动）。
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub link: LinkInfo,
    pub hid_online: bool,
    pub active_scene: Option<u8>,
    pub scenes: Vec<SceneView>,
    pub devices: Vec<DeviceState>,
    pub bindings: Vec<BindingView>,
    pub scene_mode: String,
    pub artpi_ip: String,
}

pub struct AppState {
    pub map: Mutex<HomeMap>,
    pub mgr: Mutex<SceneManager>,
    pub link: Mutex<LinkInfo>,
}

impl AppState {
    pub fn new(map: HomeMap, peer: String) -> Self {
        let mgr = SceneManager::new(map.scenes.clone());
        AppState {
            map: Mutex::new(map),
            mgr: Mutex::new(mgr),
            link: Mutex::new(LinkInfo {
                up: false,
                peer,
                pending: 0,
            }),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let link = self.link.lock().unwrap_or_else(|e| e.into_inner());
        let active = mgr.active();
        Snapshot {
            link: link.clone(),
            hid_online: crate::hid::host_action::hid_online(),
            active_scene: active,
            scenes: scene_views(&map.scenes, active),
            devices: mgr.states(),
            bindings: binding_views(&map.bindings),
            scene_mode: map.scene_mode.clone(),
            artpi_ip: map.artpi_ip.clone(),
        }
    }
}

fn scene_views(scenes: &[SceneDef], active: Option<u8>) -> Vec<SceneView> {
    scenes
        .iter()
        .map(|s| SceneView {
            id: s.id,
            name: s.name.clone(),
            command_count: s.commands.len(),
            active: Some(s.id) == active,
        })
        .collect()
}

fn binding_views(bindings: &[Binding]) -> Vec<BindingView> {
    bindings
        .iter()
        .map(|b| BindingView {
            uuid: b.uuid.clone(),
            action: b.action.label(),
        })
        .collect()
}
