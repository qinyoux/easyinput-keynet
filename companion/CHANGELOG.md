# CHANGELOG

> 开发者视角。用户视角摘要见 README「版本历史」。
> 版本号真源：`package.json`；`Cargo.toml` / `tauri.conf.json` 由 `npm run version:sync` 同步。

## [0.2.0] - 2026-10-15

真机联调首日。板子侧按键→ PC → ART-Pi 全链路打通（板端实测发出
`...000000000101`），同时修掉三个**静默失败**类缺陷。

### 新增

- **4 个诊断/生成工具（`src-tauri/src/bin/`）**——本轮的核心产出不是功能，
  是**让机制自己报错**：
  - `hid-probe.rs`：HID 探测器。打印所有 0x11 报告不过滤不丢弃 + 48B raw hex
    转储 + `[5..41]` vs `[6..42]` 两种候选偏移对照。用于实板确认 UUID 偏移
    与板内实际配置（**设备内配置 ≠ 磁盘配置**，必须实板探测）。
  - `check-config.rs`：配置加载自检。独立跑 `serde_json::from_str::<HomeMap>`
    打出**错误原文**，并打印 `load()` 实际生效的 `artpi_ip`。`config_store::load`
    在解析失败时**静默回退 `HomeMap::default()`**，只看它自己的一行 warn
    是找不到根因的。
  - `export-scenes.rs`：导出 Rust 内置 `default_scenes()` 为 JSON。
    **`scenes` 字段从此不再手写**——手写必错（详见下方「修复」第 2 条）。
  - `udp-selftest.rs`：把 `UdpLink` 从 Tauri/线程/配置里剥出来裸跑，
    判定表写在文件头（能发⇒查宿主侧；不能发⇒查运行环境）。
- **`tools/` 三个 Python 工具**（路线 A：借 easyop 的 0x10 通道刷板子 keymap，
  固件零改动）：`make_smart_home_profile.py`（追加 easyop 方案，路径从
  `tauri.conf.json` identifier反推 + 存在性校验）、`make_home_map.py`
  （scenes 从 `export-scenes` 导入）、`verify_profile_push.py`（23 项下发前
  干跑断言：主机透传 / 固件格式判据 / 载荷 ≤2048B / 跨程序 UUID 集合相等 /
  easyop 保留段撞车 / **Companion `kind` 必须在 Rust `Action` 枚举里**）。

### 修复

1. **`cargo run` 因多 bin 而失败**（本轮引入的回归）。新增 `hid-probe` 后
   工程有两个 bin，`npm run tauri dev` 报 `could not determine which binary
   to run`。修：`Cargo.toml [package]` 加 `default-run = "easyinput-companion"`。
   **新增 bin 时必须同步复核这一行。**
2. **`home_map.json` 字段名对不上导致配置静默失效**。手写的
   `{"scene":1,"label":...,"ops":[{"op":"device_on"}]}` 与 Rust 真源
   `SceneDef{id,name,commands}` + `SceneCmd{#[serde(tag="op",
   rename_all="snake_case")]}`（真值是 `light_on_off`/`thermo_target`）
   不匹配 ⇒ serde 整份 `HomeMap` 失败 ⇒ `load()` 静默回退默认 ⇒
   `artpi_ip` 变 `192.168.1.10` ⇒ **帧全发给空气，日志里只有一行 warn**。
   修：机制固化（`export-scenes` + `check-config`），不再手写 scenes。
3. **Windows 读超时被当成错误**（真 bug）。`set_read_timeout(50ms)` 到期后
   `recv_from` 返回 `WSAETIMEDOUT(10060)`，其 `ErrorKind` 是 `TimedOut`
   而**不是** `WouldBlock` ⇒ 每 50ms 产出一条「UDP 接收错误」⇒
   **UI 日志面板每秒被刷 20 条，真故障信息全被埋**。
   修：`recv_once` 同时接受 `WouldBlock | TimedOut` 并安静返回。
   补 2 条回归测试 + 给 `LinkEvent` 加 `Debug` derive（测试断言需打印事件列表）。

### 测试

`cargo test --lib` **56/56 绿**（原 54 + 新增 2 条回归测试），0 警告。

### ⚠️ 未完成

- ART-Pi **实板**联调未做（本机无 ART-Pi，当前链路对面是 `artpi-sim`模拟器）。
  设备引脚仍为 `-1` 占位，继电器不会实际动作。
- `scene_mode="remote"` 待 ART-Pi M4（`on_scene` 目前是 `TODO(M4)` 桩）后才能实测。
- 4 个诊断 bin 中的 `export-scenes` / `check-config` / `udp-selftest` 目前是
  **命令行工具**，未接入 UI。

## [0.1.0] - 2026-10-14

### 新增

- **方案 D MVP 首个可运行版本**：EasyInput 开发板（固件零改动）→ PC Companion → ART-Pi 全链路。
- V2.3 统一帧编解码（`v23/frame.rs`）：与 ART-Pi `sh_frame.h` 逐字段对齐；
  `SeqWindow` 逐位复刻 ART-Pi 16 槽 seq 去重语义（含"窗口外重置"规则）。
- UDP :17335 链路（`v23/udp_client.rs`）：`RetransmitQueue` 纯逻辑层（200ms 重发、
  最多 3 次重传、**重传换新 seq**）+ `UdpLink` IO 层（收包/重传/心跳 800ms/3s 判活）。
- HID 三件套移植（`hid/`）：Win32 SetupDi 枚举 + OVERLAPPED 读 0x11 kind 0x05
  报文 + 规范 UUID 校验（自 easyop 移植，去掉下发路径）。
- 家居场景状态机（`home/scene.rs`）：8 场景 × 命令展开、设备状态跟踪、
  编码器调光/翻转、单设备 toggle；本地乐观更新 + ART-Pi 回传权威覆盖。
- 桥接层（`home/bridge.rs`）：UUID → Action → V2.3 命令 → 通道；未绑定 UUID
  显式报错不静默。
- 配置系统（`config_store.rs` + `config/home_map.json`）：ART-Pi IP / 场景模式
  （expand|remote）/ UUID 绑定表 / 场景表；加载清洗 + 落盘。
- React 前端（WorkBuddy 浅色主题）：链路双灯、8 场景条、6 设备卡片、
  命令注入调试台（预设芯片 + 裸命令）、事件日志（500 条环形）、配置面板
  （UUID 点击复制、IP 热切换）。
- `scripts/version.mjs`：版本号 check/sync/bump（零依赖）。

### 复刻继承的工程决策（来自 easyop）

- `[build-dependencies] indexmap = { version = "1.9", features = ["std"] }`：
  修 rustc 1.98 + schemars 0.8.22 编译失败（挂 build-dependencies 才有效）。
- `[patch.crates-io] tao = vendor/tao`：Rc→Arc，修并发 clone AppHandle 的
  UB abort（0xc0000409 BEX64）；附 `vendor/tao/README-easyop-patch.md`。
- `[lib] crate-type = ["rlib"]`：只出 Windows 桌面版，不编 staticlib/cdylib。
- `custom-protocol` feature 必须随 Tauri CLI 出包（否则双击 exe 连 localhost）。

### 测试状态（2026-10-14 宿主测试）

- `cargo test`：**54/54 全绿**（帧编解码/CRC 金标准 0x29B1/坏帧拒绝/SeqWindow
  复刻 ART-Pi 语义/重传递增 seq/重传预算/场景展开/设备 toggle/配置清洗/保存回读）。
- `cargo build`（release）：**0 警告 0 错误**。
- `npm run build`（tsc + vite）：通过，产物 153KB JS + 7.5KB CSS + 4 枚 Assistant 字体。
- `npm run tauri build`：出包成功 `src-tauri/target/release/easyinput-companion.exe`（9.4MB）。
- 运行时冒烟：exe 启动 12s 存活（tao Arc 补丁 + 双线程装配 + 配置加载验证）。

### ⚠️ 未完成

- **ART-Pi 实板联调未做**（本机无 ART-Pi 硬件在场）：链路层已按协议单测覆盖，
  真机 Ack/状态回传待接板验证。
- ART-Pi 侧 `on_scene` 为 TODO(M4) 桩 ⇒ Companion 默认 expand 模式展开场景；
  remote 模式已预留未实测。
- 无 Wi-Fi 自动发现：ART-Pi IP 需手动配置（默认 192.168.1.10）。
- `desk_led` 0x16 灯效通道**实板未验证**（固件 flow/进展.md 原话），本工程不涉及。

### 下一步

1. `cargo test` 全绿 + `npm run build` 通过；
2. 接 ART-Pi 实板联调（继电器 + SHT30）；
3. 录演示视频（KEY1/KEY2 场景 + 调试台注入 + 链路指示灯）。
