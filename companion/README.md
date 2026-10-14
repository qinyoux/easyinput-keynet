# EasyInput Companion · 智能物联网中控 PC 侧（第8课大作业 · 方案 D MVP）

> EasyInput 开发板 + PC Companion + ART-Pi = 生活向智能物联网中控平台。
> **ESP32-S3 固件零改动**：复用 easyop 同款固件的 HID 0x11 Host Action 通道，
> 场景状态机与 V2.3 帧协议全部在 PC 侧实现。

## 系统架构

```text
┌──────────────┐  USB HID   ┌────────────────────┐  UDP :17335  ┌─────────────────┐
│ EasyInput    │ 0x11 kind  │  PC Companion      │  V2.3 帧     │ ART-Pi          │
│ 开发板       │ 0x05       │  (本工程)          │  CRC16+seq   │ (RT-Thread)     │
│ 8键+编码器   │ ─────────► │  HID监听→场景机    │ ───────────► │ 6设备/继电器/   │
│ (固件不改)   │  36B UUID  │  →重传队列→UDP     │  Ack 200ms   │  SHT30 温控     │
└──────────────┘            └────────────────────┘              └─────────────────┘
                                     │
                    配置源：easyop（0x10 通道绑定 host_action:<uuid>）
```

## 快速开始

```bash
cd companion
npm install
npm run tauri dev        # 开发模式（前端 :1420 + Rust 热重载）
npm run tauri build      # 出包（必须走 CLI，见 Cargo.toml custom-protocol 说明）
npm run version:check    # 收工校验：版本号三处一致
```

宿主测试：

```bash
cd companion/src-tauri
cargo test               # 帧编解码 / CRC 已知值 / seq 窗口 / 重传递增 / 场景机 / 配置清洗
```

## 演示链路（无实体 ART-Pi 也能跑）

1. 启动 Companion → 顶栏两盏灯：HID（板子在线）/ ART-Pi（离线也允许操作，命令进重传队列）；
2. 点「命令注入」预设芯片（如"主灯 50%"）→ 事件日志出现"发送/失败重传"记录；
3. 接上 ART-Pi（同网段，IP 在配置页改）→ 链路灯变绿，Ack 逐条确认；
4. 按板子 KEY1（回家）/ KEY2（影院）→ 与点场景按钮完全同路径。

## 协议要点（与 ART-Pi `sh_frame.h` 逐字段对齐）

| 项 | 值 |
|---|---|
| 帧格式 | `5A A5 ver=02 type cmd len seq(2,大端) sid payload CRC16(小端)` |
| CRC | CRC-16/CCITT-FALSE（init 0xFFFF, poly 0x1021），覆盖 9+len 字节 |
| 端口 | UDP :17335 |
| 命令 | 0x20 查询 / 0x30 心跳 / 0x40–0x46 设备 / 0x47 场景 / 0x48 查全部 |
| 状态 | Ack 载荷 N×7B：`id,onoff,level,temp_x10_lo,hi,err,rsv` |
| 可靠性 | 200ms 重发 ≤3 次；**重传必须换 seq**（ART-Pi 16 槽去重窗口会吃同 seq 帧） |
| 判活 | PC 侧 3s 无回帧判链路断；ART-Pi 侧 1s 无帧判失联 |

## 目录

```text
companion/
├── src/                     # React 前端（WorkBuddy 浅色主题）
│   ├── components/          # LinkStatus / SceneBar / DeviceGrid / CommandInjector / ConfigPanel / EventLog
│   └── styles.css
├── src-tauri/src/
│   ├── v23/frame.rs         # V2.3 帧编解码 + SeqWindow（复刻 ART-Pi 语义）
│   ├── v23/udp_client.rs    # RetransmitQueue（纯逻辑可测）+ UdpLink
│   ├── hid/                 # Win32 HID 三件套（自 easyop 移植）
│   ├── home/scene.rs        # 8 场景状态机 + 设备状态跟踪
│   ├── home/bridge.rs       # UUID → 动作 → 命令 桥接层
│   └── config_store.rs      # home_map.json 读-改-存
├── config/home_map.json     # 默认映射模板（= 首次运行的内置表）
└── scripts/version.mjs      # 版本单真源 check/sync/bump
```

## 已知边界（MVP 裁剪，详见 CHANGELOG）

- 场景在 **Companion 侧展开**成设备命令（ART-Pi `on_scene` 尚为 TODO(M4) 桩）；
  `scene_mode="remote"` 预留切换。
- ART-Pi 实板联调待接板验证（协议层已 54/54 单测覆盖）。
- 无 Wi-Fi 自动发现：ART-Pi IP 需手动配置（默认 192.168.1.10）。
- 测试基线：`cargo test` 54/54 绿、`cargo build` 0 警告、`npm run build` 通过、
  出包 + 12s 运行时冒烟通过。

## 版本历史

见 `CHANGELOG.md`。
