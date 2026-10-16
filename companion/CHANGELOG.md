# CHANGELOG

> 开发者视角。用户视角摘要见 README「版本历史」。
> 版本号真源：`package.json`；`Cargo.toml` / `tauri.conf.json` 由 `npm run version:sync` 同步。

## [0.3.0] - 2026-10-16

### 修复
- **★ EasyInput 触发场景时触摸屏高亮不同步**（真机反馈驱动，用户原话：
  「EasyInput触发同一场景，高亮没有同步，但是灯和上位机是同步了」）。
  **根因不在 ART-Pi，在Companion 默认场景模式**：初版因为 ART-Pi `on_scene`
  还是 `TODO(M4)` 桩，默认走 `scene_mode="expand"`——Companion 把场景拆成
  4–6 条设备命令逐条发，**ART-Pi 从未收到 0x47**，`sh_scene_execute()` /
  `sh_ui_set_active_scene()` 一次都没被调用。灯和上位机确实同步了（那几条命令
  真执行了），所以这个 bug 极具迷惑性：不是偶发，是**必然**。
  ART-Pi 侧 `on_scene` 已于本日落地为真实现，默认模式随之改为 `remote`
  （只发 1 个 0x47，板子本地展开 + 置位高亮）。
- **老配置迁移**（`config_version` v1 → v2）：只改默认值是不够的——用户磁盘上
  的 `home_map.json` 明写`"scene_mode": "expand"`，加载时被原样保留。迁移把
  版本号落后的老配置拉回 `remote`；已是 v2 的用户**显式选 expand 会被尊重**
  （老固件退路依然可用，不会被反复改回）。

### 新增
- `SceneManager::activate_remote()`：remote 模式只发 1 个 0x47，但本地**照样推演
  设备状态**（复用 `apply_to`）——上位机设备卡片不必空等状态帧回来才变。
  原`set_active()` 只置激活位、不推演状态，已替换。
- `HomeMap::config_version` 字段 + `migrate()`，v1→v2 一次性迁移。
- 6 条回归测试（62 → 68）：默认模式必须只发 1 个 `0x47`、remote 模式本地状态
  推演、未定义场景必须报错且不发包、老配置 expand→remote 迁移、v2 显式expand
  被尊重、remote 模式不被迁移改动。
- 证据：`证据/scene_walk_remote_mode.log`（8 场景 9 包全 Ack）。

### 变更
- **默认 `scene_mode`：`expand` → `remote`**。`expand` 保留为老固件退路，但**在
  该模式下触摸场景栏高亮必然不同步**（这是它的固有语义，不是 bug）。
- 场景 8 演练耗时从约 12s 降到 1.6s（4–6 个包 → 1 个包），且不再受真机实测的
  `CMD_GAP_MS=150ms` 连发节流约束——本地展开在单帧内同步完成，不经收包通道。

### 教训
- **"代码路径存在" ≠ "路径会被走到"。** 上一轮把「触摸/PC/FinSH 三路高亮单点
  收口」当成已验证结论汇报，实际只验了 ART-Pi 侧路径存在，没验 Companion
  默认配置会不会发0x47。主机单测 114 断言全绿也照样漏了——测的是 ART-Pi，
  缺陷在 PC 侧。**跨端功能的"已同步"必须有跨端实测证据。**

## [0.2.6] - 2026-10-16

### 修复
- **`SeqWindow` 镜像对齐 ART-Pi seq 去重修复**（测试专用代码，PC 运行路径
  零变化——PC 侧不做去重，重传始终换 seq）：ART-Pi `frame_codec.c` 旧算法把
  `diff >= WINDOW` 一律当"新流重置"，递增流每帧重置、位图永远只剩 1 位，
  last-1 重放拦不住。现按回绕差分符号分流（领先前移 / 出窗重置 / 同 seq 拒），
  `SeqWindow` 逐位复刻修复后的语义。
- `seq_window_matches_art_pi_semantics` 旧断言钉的正是 bug 行为
  （"101 之后的 100 会被放行"），已改为"必被吃"。

### 新增
- 3 条 seq 窗口回归测试（镜像 ART-Pi host_test 用例）：递增流重放全拦、
  65535→0 回绕无缝、对端重启回落小值不锁死。cargo test 59 → 62。
- **`artpi_sim.py` 的 `SeqWindow` 同步为修复后语义**（此前仍是旧算法，
  模拟器自称「逐位复刻 `sh_frame_seq_fresh`」却与修复后的真机漂移）；
  新增 `reset()` 对齐 `sh_frame_seq_reset()`。测试 25 → 34 用例：
  递增流重放/回绕/大跳变/reset 四组回归 + **旧算法负面实证**
  （把旧算法复刻进测试文件，证明新用例咬得住 bug）。
- 模拟器场景 0x47 与 ART-Pi `sh_scene.c` 语义同步为真实现（此前为 TODO(M4)
  桩：回 Ack 不执行），8 场景 30 条命令三端一致，由
  `tools/scene_parity.py` 程序化校验。

### 文档
- `companion/README.md` 已知边界：场景「Companion 侧展开 + ART-Pi 桩」改写为
  「两路并存且语义一致」，测试基线 54/54 → 62/62。
- `artpi-sim/README.md`：复刻表与用例数（18 → 34）同步。

## [0.2.5] - 2026-10-15

### 新增
- **自适应心跳（双档）**：钦友反馈固定 800ms 心跳把 ART-Pi 串口刷满
  （每帧 `rt_kprintf`，75 条噪音/分钟），调试信息全被淹没。改为：
  - **探测档 300ms**：未连通（含启动）或链路可疑时，快速找板子；
  - **保活档 5000ms**：连通且无异常时，串口噪音降到 12 帧/分钟（-84%）；
  - **无应答即升档**：任何命令重传耗尽（`Due::Failed`）置 `suspect`，
    心跳立即回探测档；收到任何回帧即解除。
- 3 条单测钉死档位规则：双档切换矩阵、`HEARTBEAT_IDLE_MS + 重传余量 <
  LINK_DEAD_AFTER_MS` 联动校验（防「判死→探测→复活→放缓→又判死」振荡）、
  Failed 置可疑/回帧解除的生命周期。

### 变更
- `LINK_DEAD_AFTER_MS` 3000 → 8000：**必须与心跳档联动**。保活档下单个
  Ack 丢失要等到下一跳（5s 后）才有回帧，旧 3s 阈值会在每个保活间隙误判
  死链造成振荡。代价：断连后 UI 最多 8s 才显示离线（此前 3s）。
- 板子侧「1s 无帧判 PC 失联」只影响板子在线指示，不影响命令执行
  （命令收帧独立处理，真机 8 场景 31 命令全 Ack 已验证）。

### 修复
- **双击 exe「无法访问」**：根因是双击了 `target/debug/` 下的 dev 产物——
  它加载 `http://localhost:1420`（vite 开发服务器），不开 `npm run tauri dev`
  必然无法访问。正确用法二选一：
  ① 开发调试：`cd companion && npm run tauri dev`；
  ② 直接双击：用 `target/release/easyinput-companion.exe`（0.2.5 起重建，
     前端资源已内嵌，双击即用，无需任何服务器）。

## [0.2.4] - 2026-10-15

### 修复
- `spawn_udp_thread` 命令节流引入的 `unused_assignments` 警告：
  `sent_this_round` 初值在循环体第一行的每轮重置之前永不被读。改为
  不赋初值声明（循环首行赋值先于一切读取）。0.2.2 引入节流时漏掉。

### 流程自纠
- 0.2.2/0.2.3 的收工校验用 `grep -cE "^warning: (unused|unreachable|variable)"`
  统计警告数——模式过窄，漏掉了 lint 名 `unused_assignments`，
  导致"0 警告"的结论不成立（`tauri dev` 后台日志才暴露）。
  **零警告判据必须用全量 `grep -c "^warning"`**，白名单式过滤等于自己给
  自己放水。本版起修正。

## [0.2.3] - 2026-10-15

### 新增
- **ART-Pi 真实 GPIO 绑定（验证期 LED 方案）**：`MyART-PI/applications/main.c` 三个占位
  pin（-1）绑定实脚——PH3→主灯(id1)、PB13→插座1(id5)、PB2→插座2(id6)。选择依据是
  8 场景 31 条命令的设备频次：主灯 10 条/7 场景、插座1 3 条、插座2 2 条，绑完后
  7/8 场景有可见反馈。灯带/风扇/温控保持 -1 软件桩（PWM/SHT30 硬件未接）。
  极性 RT_FALSE（GPIO 高电平点亮，LED 直脚常见接法），注释写明反转方法。
  固件已 scons 编译通过（text 426,996 + data 2,372 + bss 61,644），
  反汇编验证三个 pin 常量（29/18/115）已入 main.o。**烧录须钦友确认，未烧。**
- `types.ts::formatTemp() + TEMP_X10_BAD`：温控哨兵识别。ART-Pi 在
  `read_temp()` 失败时上报 `SH_THERMO_TEMP_X10_BAD(-1000)` + err=1，语义是
  「SHT30 没读到」而非 -100℃ 实测。UI 显示「传感器未接」并置灰——
  把哨兵值渲染成「-100.0℃」是把「没有数据」谎报成「荒谬的数据」。

### 变更
- 演示脚本 Checklist 加第 9/10 条（LED 映射自检、预期显示），新增「硬件映射与
  烧录」小节（含 scons shebang 坑与 `.sconsign` TypeError 的正确处置），
  S2 口播改为 LED 现实描述，翻车表补哨兵/极性/软件桩三条。

## [0.2.2] - 2026-10-15

**ART-Pi 真机首次闭环**，并修掉一个只有真机才暴露的系统级约束。

### 真机联调结果

ART-Pi（`D:\RT-ThreadStudio\workspace\MyART-PI`，RT-Thread + AP6212 WiFi，
连 `vinno-guest`，DHCP 拿地址）与 Companion 建立 UDP :17335 连接：
`scene-walk` 打 **10.10.21.29**，8 场景 31 条命令全部 Ack，连跑 3 次零丢失。
证据：`证据/scene_walk_real_artpi.log`。

### 修复

1. **同一场景的多条命令连发会导致 ART-Pi 随机丢包**（真机实测，产品级问题）。
   `spawn_udp_thread` 的 `while let Ok(cmd) = rx.try_recv()` 会把通道一次性
   排空，一个场景展开出的 2–6 条命令在同一轮里无间隔发出。实测「离家」
   场景 6 条命令 ×5 轮：

   | 命令间隔 | Ack 率 |
   |---------|--------|
   | 0ms（连发） | 5/30（16%） |
   | 150ms | 30/30（100%） |
   | 250ms | 30/30（100%） |

   丢包时**重传也救不回来**（3 次重传全丢）——板子丢包属接收缓冲溢出型，
   200ms 后重发同样进不去。所以必须从源头降速，不能依赖重传兜底。
   修法：`constants::CMD_GAP_MS = 150`，`spawn_udp_thread` 每轮内多条命令
   之间让出间隔。根因在板子侧（`ctrl_server.c` 每包 `rt_kprintf` 阻塞收包
   线程 + AP6212 接收缓冲），而路线 D 前提是固件零改动，故由 PC 侧迁就。
   150ms 的依据是实测；真人按键天然 >200ms 间隔，这是对齐真实使用节奏。

### 新增

- **`tools/artpi_probe.py`：ART-Pi 协议级探测器**。给候选 IP 发 0x30 心跳，
  **回 Ack 者即 ART-Pi**——不靠 MAC 前缀猜（DHCP 下 IP 不固定，AP6212 的
  MAC 也难与同网设备区分）。支持 `--ip` 单测 / `--range` 全网段扫描。
  - 踩坑：Windows `arp -a` 输出是 **GBK**，按 UTF-8 解会抛
    UnicodeDecodeError（"动态"字段落在非 ASCII 区）。

### 已知边界（真机侧观察，未改）

- **温控上报 `-100.0℃`**：`SH_THERMO_TEMP_X10_BAD (-1000)` 是**传感器无效
  哨兵值**，`thermo_ctrl.c` 在 `read_temp()` 失败时置 `err=1` + 该值。
  即 ART-Pi 在如实上报"SHT30 没读到"，不是解析错误。Companion 侧目前会
  原样显示 -100.0℃，**未做哨兵值识别**（显示为"传感器异常"更诚实）。
- **状态帧设备顺序是注册序非 id 序**（实测 `5,6,1,2,3,4`）：真源
  `on_query_status` 用 `dev_registry_get_by_index(i)` 遍历。Companion 按
  条目内 id 字段取值，故不影响功能。
- **ART-Pi 的 seq 去重窗口对递增序列失效**：`sh_frame_seq_fresh` 用
  `diff = seq_last - seq`，seq 递增时 diff 恒 ≥ 16 ⇒ 永远走"窗口外"分支
  并重置窗口 ⇒ 同 seq 重放挡不住。这是板子侧问题，**本次未改**（用户要求
  先不动 ART-Pi），已记录待后续处理。

## [0.2.1] - 2026-10-15

8 键全场景实机验证通过后的加固轮。**产品功能零变更**——本轮产出全部是
「让演示可复现」和「让测试不自欺」的证据工具与断言修正。

### 新增

- **`bin/scene-walk.rs`：8 场景顺序演练器（演示用证据生成器）**。走
  **产品代码原样的路径** `scene_uuid(n) → bridge::dispatch() → UdpLink`，
  按序把 8 个场景打进 UDP，因此「本bin 跑通 = 真机链路跑通」。
  - 解决的问题：演示视频要录「按 KEY1–KEY8 依次生效」，手按 8 次不确定——
    漏按、错序、忘词都会让视频作废。演练器产出可归档文本，重录不重演。
  - 三条硬判据：配置目录不存在直接 `exit 1`、生效场景数 ≠ 8 直接
    `exit 1`、一条 Ack 都没有直接 `exit 1`。**宁可失败也不产出假证据**
    （`config_store::load` 解析失败会静默回退默认表，这是 0.2.0 踩过的坑）。
  - 实测：8 场景 31 条命令全部 Ack，产物 `证据/scene_walk.log`。
- **`artpi_sim.py --log <path>`**：模拟器日志同时写文件（追加）。
  屏幕日志随终端关闭而消失，而录视频、写作业都需要可归档物证。
- **`Client.temp_now()`：同步读温控温度**。走 0x20「查询单设备」的
  请求-应答，取代原先「sleep → 读状态帧」。
- **`Client.drain()`**：清空接收队列存量。
- **回归断言「temp_now 读的是同步查询而非状态帧」**（`inspect.getsource`
  自检）：若有人把 `temp_now` 改回读状态帧，此断言立即失败。

### 修复

1. **模拟器测试温控用例间歇性假失败**（`artpi_sim/test_client.py`，
   实测 `17 通过 / 1 失败`，可复现）。三处叠加，**产品代码完全正常**：
   - **依赖初始状态**：用例假定室温 24.0℃ 再测「制冷必降」。但共享常驻
     模拟器时，室温是历史遗留值（本轮实测已被 `scene-walk` 推到 20.0℃
     下限）⇒ `t1 < t0` 必然不成立。
   - **依赖状态帧时序**：状态帧有「变更即时」与「5s 周期」两套节拍，
     命令落在周期边界附近时下一帧要等满 5s。原写法 sleep 2s 后
     `wait_status` 超时，返回队列里的**旧帧**——实测恒为 `→ 200`。
   - **固定等待不足以跨过距离**：步进 0.5℃/s，从 20.0℃ 爬到 28℃ 需 16s，
     而 sleep 2s 只走 10 个单位 ⇒ 起点原地不动，制冷段 `200 → 200`。
   - 修法：断言改用**相对变化**（比自己的起点高/低）+ **轮询到温度真正
     离开起点**而非固定等待 + 读值改走同步查询。方向单调性由模拟器
     max/min 钳位保证，只可能慢，不会反向。
2. **测试套件依赖全局状态、不自洽**（上一条的本质）。修后同一常驻模拟器
   连跑 5 次均 20/20，换端口连跑亦 20/20。

### 验证

- `cargo test` 56/56绿 · `artpi_sim/test_client.py` 20/20（连跑 5 次幂等）
- `scene-walk` 实跑：8 场景 31 命令全 Ack，链路存活、未确认 0

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
