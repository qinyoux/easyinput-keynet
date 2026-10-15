//! 配置加载自检：把 `home_map.json` 真喂给 `config_store::load`，验证**没被静默回退**。
//!
//! **为什么必须有这个 bin**：`config_store::load` 在文件缺失或反序列化失败时
//! **不返回错误**，而是静默回退 `HomeMap::default()`（artpi_ip 变
//! `192.168.1.10`）。2026-10-15 真机联调就踩了这个坑：Python 生成的
//! `scenes[].ops` 用的是 `device_on`/`set_temp`，而 Rust 的 `SceneCmd`
//! 是 `light_on_off`/`thermo_target` ⇒ serde 整份失败 ⇒ 回退默认 ⇒
//! 帧全发给空气，日志里只有一行 `warn`。
//!
//! 本 bin 加载后把关键字段打出来，**任何静默回退都能一眼看出**：
//! artpi_ip 不是配置里写的那个 ⇒ 回退了。
//!
//! 用法：`cargo run --bin check-config`

use companion_lib::config_store::HomeMap;

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            // 默认取 Windows 的 %APPDATA%\com.easyinput.companion
            let base = std::env::var("APPDATA").expect("APPDATA 未设置");
            std::path::Path::new(&base).join("com.easyinput.companion")
        }
    };
    let path = dir.join("home_map.json");

    println!("配置文件 = {}", path.display());
    if !path.exists() {
        println!("[FAIL] 文件不存在");
        std::process::exit(1);
    }
    println!("字节数 = {}", std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));

    // 先用 serde_json 独立解析一次，把错误原文打出来——
    // load() 会吞掉错误，只留一行 warn，看不到真正原因。
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<HomeMap>(&text) {
            Ok(_) => println!("[OK] serde 解析成功（字段名/op 值都对得上）"),
            Err(e) => {
                println!("[FAIL] serde 解析失败：{e}");
                println!("       ⇒load() 会静默回退内置默认，artpi_ip 变192.168.1.10");
                println!("       ⇒ 离线调用的帧全部发给空气，且日志里只有一行 warn");
                std::process::exit(1);
            }
        },
        Err(e) => {
            println!("[FAIL] 读文件失败：{e}");
            std::process::exit(1);
        }
    }

    let map = companion_lib::config_store::load(&dir);
    println!();
    println!("--- load() 实际生效的配置 ---");
    println!("artpi_ip   = {}", map.artpi_ip);
    println!("artpi_port = {}", map.artpi_port);
    println!("scene_mode = {}", map.scene_mode);
    println!("bindings   = {}", map.bindings.len());
    println!("scenes     = {}", map.scenes.len());

    if map.scenes.is_empty() {
        println!("[WARN] 场景为空（sanitize 会用 default_scenes 填充，不算回退）");
    }
    println!();
    println!("[结论] serde 解析成功 ⇒ 配置真的生效了，没有静默回退。");
}