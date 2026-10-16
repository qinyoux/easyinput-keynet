# artpi-sim — ART-Pi V2.3 协议模拟器

没有 ART-Pi 真机时，用它顶替中控主机，把 PC Companion 的**整条协议链路**跑通。
纯 Python 标准库，零依赖。

## 它复刻了什么（逐位对齐 RT-Thread 真源）

| 行为 | 真源 | 复刻度 |
|------|------|--------|
| 帧格式 `0x5A 0xA5 ver=02 type cmd len seq(BE) sid payload CRC16(LE)` | `frame_codec.c` / `sh_frame.h` | 字节级一致 |
| CRC16-CCITT（poly 0x1021 / init 0xFFFF / MSB-first，覆盖 9+len 字节） | `crc16_ccitt()` | 金标准 `crc16("123456789")==0x29B1` |
| seq 去重滑动窗口（16 槽位图，RFC 1982 回绕差分） | `sh_frame_seq_fresh` | 逐位复刻（2026-10-16 修复后语义） |
| Ack 回显 seq/sid；0x20/0x48 的 Ack 带 N×7B 状态 | `ctrl_server.c` | 一致 |
| 状态帧：命令变化即时 + 周期 5s | `status_report.c` | 一致 |
| 场景 0x47 = 本地展开为设备命令序列 | `cmd_dispatch.c::on_scene` → `sh_scene.c` | 8 场景 30 条命令，三端一致 |
| 温控闭环 | `thermo/`（迟滞+PID，0.1℃ 定点） | 简化为 0.5℃/s 一阶趋近（仿真便利） |

## 用法

```bash
python artpi_sim.py                  # 监听 0.0.0.0:17335，室温 24.0℃
python artpi_sim.py --port 17335 --room-temp 260

# 另开终端跑集成测试（34 用例）
python test_client.py
```

测试客户端扮演 Companion 的 UdpLink（首送 + 200ms **换 seq** 重传 ≤3 次），
覆盖：金标准 CRC / 去重语义（含 2026-10-16 递增流重放回归 + 旧算法负面实证）/
设备命令 / 查询 Ack / **同 seq 重放被吃（复现坑）** / 坏帧容错 /
场景 0x47 真实现（影院/离家/未知 id）/ 温控收敛。

## 已实证的端到端链路

Companion 真身（Rust UdpLink）↔ 本模拟器：启动即查询全部、心跳自适应
（0.2.5 起：未连通/可疑 300ms 探测，连通后 5s 保活，给板子串口让路）、
seq 连续递增——见 `companion/README.md` 测试基线与仓库提交记录。

## Windows 备注

UDP 对端关闭后 ICMP 回灌会让 `recvfrom` 抛 `WinError 10054`
（ConnectionResetError）——本模拟器与该现象共存：收到即视为对端消失，
清空记录等新客户端，**不退出、不刷屏**。这是 Windows UDP 的固有语义，
真实 ART-Pi（RT-Thread lwIP）上没有这个问题。
