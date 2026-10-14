# CHANGELOG

> 开发者视角。用户视角摘要见 README「版本历史」。
> 版本号真源：`package.json`；`Cargo.toml` / `tauri.conf.json` 由 `npm run version:sync` 同步。

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
