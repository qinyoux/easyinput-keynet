//! 导出 Companion 内置默认的 `scenes` 真源（JSON 片段）。
//!
//! **为什么需要它**：`home_map.json` 的 `scenes` 字段类型是 `Vec<SceneDef>`，
//! `SceneDef.commands` 是 `Vec<SceneCmd>`，而 `SceneCmd` 是带 `#[serde(tag="op",
//! rename_all="snake_case")]` 的枚举（`light_on_off` / `light_level` /
//! `fan_on_off` / `thermo_target` / `thermo_mode` / `socket_on_off`）。
//!
//! 手写这份 JSON 必然踩坑：字段名/`op` 值对不上 → serde 整份 `HomeMap`
//! 解析失败 → `config_store::load` **静默回退内置默认**（artpi_ip 变回
//! `192.168.1.10`）→ 帧发给空气，日志里只有一行 warn。
//! 这个坑真实发生过一次（2026-10-15）。
//!
//! 做法：让 Rust 自己序列化，Python 只做拼接。**真源唯一，且永不漂移。**
//!
//! 用法：`cargo run --bin export-scenes > scenes.json`

use companion_lib::home::scene::default_scenes;

fn main() {
    let scenes = default_scenes();
    println!("{}", serde_json::to_string_pretty(&scenes).expect("序列化内置场景失败"));
}