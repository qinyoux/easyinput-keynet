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

用户视角摘要（开发者视角见 `CHANGELOG.md`）。

### 0.2.2 — 2026-10-15

**ART-Pi 真机首次闭环**：8 场景 31 条命令全部确认，连跑 3 次零丢失。

同时修掉一个只有真机才暴露的问题：**一个场景展开出的多条命令不能连发**。
之前 Companion 会把通道里的命令一次性排空，导致 ART-Pi 随机丢包——
实测「离家」场景 6 条命令连发只有 **16%** 被确认，而板子丢包后重传也救
不回来（重发同样进不去）。现在命令之间留 150ms 间隔，实测 **100%**。

这个间隔不是猜的：0ms→16%、150ms→100%、250ms→100%，取实测能稳的最小值。
真人按键天然超过 200ms，所以这只是对齐真实使用节奏。

> 根因在板子侧（每收一包就打印日志，串口阻塞了收包线程），但本项目的
> 前提是固件零改动，所以由 PC 侧让出节奏。

**已知边界**（真机侧观察，未改）：温控会上报 `-100.0℃`，这是板子在说
「SHT30 没读到数据」的哨兵值，不是解析错误；Companion 目前原样显示，
尚未识别成「传感器异常」。

### 0.2.1 — 2026-10-15

**8 个按键 → 8 个场景全部实机验证通过**，设备状态随之变化。这一版
**产品功能零变更**，产出的是「让演示可复现、让测试不自欺」的证据工具。

- **`scene-walk`：8 场景顺序演练器**。录视频不用赌「手按 8 次都按对」——
  它走产品代码原样的路径把 8 个场景按序打一遍，输出可归档文本，重录不重演。
  实测 8 场景 31 条命令全部确认。
  ```bash
  cargo run --bin scene-walk -- 127.0.0.1 > ../证据/scene_walk.log 2>&1
  ```
- **模拟器日志可落盘**：`python artpi_sim.py --log证据/sim.log`。
- **修掉模拟器测试的间歇性假失败**（原 17/18）。根因三处，都表现为
  「报失败但产品代码完全正常」：用例假定室温 24.0℃、依赖状态帧到达
  时序、固定等待不足以跨过温度距离。现断言改为「与自己起点比」+
  轮询到真正变化 + 读值走同步查询。**同一模拟器连跑 5 次 20/20。**

> 这一版的教训和 0.2.0 是同一条：**静默失败**的反面是「静默通过」。
> 一个依赖全局状态、依赖时序的测试，绿的时候不能证明任何事。

### 0.2.0 — 2026-10-15

真机联调首日，**板子按键 → PC → ART-Pi 全链路打通**（板端实测发出
`...000000000101`，模拟器侧确认收到查询 + 800ms 心跳）。

这一版的主题是**让机制自己报错**。联调暴露了三处「静默失败」——
所有成功信号都正常（脚本打印成功、备份做了、进程响应正常），
只有结果不对，且日志里几乎不留痕迹：

- 配置字段名与 Rust 真源不一致 ⇒ 配置解析失败被吞掉、静默回退默认 IP
  ⇒ 帧发给空气
- Windows 读超时（10060）被当成错误上报 ⇒ 日志面板每秒刷 20 条
  ⇒ 真故障被淹没
- 诊断工具写错了配置目录 ⇒ 脚本"成功"，但目标程序根本读不到

修法不是"下次小心"，而是新增 4 个命令行工具让这类问题自己暴露：
`check-config`（配置解析自检，打印错误原文 + 实际生效的 IP）、
`export-scenes`（场景表由 Rust 导出，不再手写）、`udp-selftest`（链路裸环境
自测）、`hid-probe`（HID 报告探测器，不过滤全打印）。

测试 56/56 绿。

**未完成**：ART-Pi 实板联调（本机无板，当前对面是协议模拟器）；
设备引脚仍是 `-1` 占位，继电器不会实际动作；`scene_mode="remote"` 待 ART-Pi M4。

### 0.1.0 — 2026-10-14

方案 D MVP 首个可运行版本：EasyInput 开发板（固件零改动）→ PC Companion
→ ART-Pi 全链路。V2.3 帧编解码与 ART-Pi `sh_frame.h` 逐字段对齐，HID 三件套
自 easyop 移植，8 场景状态机 + React 界面（WorkBuddy 主题）。测试 54/54 绿。
