#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""ART-Pi V2.3 控制协议模拟器（PC Companion 联调/演示用）。

复刻 RT-Thread 侧 `MyART-PI/applications` 的协议行为，让 PC Companion
在没有 ART-Pi 真机的环境下也能全链路自测与演示：

  帧格式   : 0x5A 0xA5 ver=0x02 type cmd len seq(BE) sid payload CRC16(LE)
            CRC16-CCITT(poly 0x1021, init 0xFFFF, MSB-first) 覆盖 9+len 字节
  seq 去重 : 16 槽位图滑动窗口（逐位复刻 sh_frame_seq_fresh）
            —— 窗口内同 seq 直接丢弃（这就是"PC 重传必须换 seq"的根源）
  Ack      : cmd/seq/sid 回显；0x20/0x48 的 Ack 载荷 = N×7B 状态条目
  状态帧   : FT_STATUS 周期 5s + 命令变化即时上报（复刻 status_report.c）
  场景 0x47: ⚠️ 真源 cmd_dispatch.c 的 on_scene 是 TODO(M4) 桩——
             收帧回 Ack 但不执行。本模拟器如实复刻该行为，
             因此 Companion 默认 scene_mode="expand"（PC 侧展开）。

与真源的差异（仅仿真便利，不影响协议语义）：
  - 温控温度收敛用一阶趋近模拟（真源是迟滞+PID，0.1°C 定点）
  - 无 FinSH、无 LCD、无真实继电器/PWM；状态全在内存

用法：
  python artpi_sim.py                 # 监听 0.0.0.0:17335
  python artpi_sim.py --port 17335 --room-temp 240
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys
import time
from dataclasses import dataclass, field

# ---------------------------------------------------------------------------
# 协议常量（与 companion/src-tauri/src/constants.rs 逐值对齐）
# ---------------------------------------------------------------------------

FRAME_HEAD1 = 0x5A
FRAME_HEAD2 = 0xA5
FRAME_VERSION = 0x02
FRAME_HEADER_SIZE = 9
FRAME_PAYLOAD_MAX = 239
FRAME_MAX = 250
FRAME_SEQ_WINDOW = 16
STATUS_ENTRY_SIZE = 7

FT_CMD = 1
FT_STATUS = 2
FT_ACK = 3

CMD_QUERY_STATUS = 0x20
CMD_HEARTBEAT = 0x30
CMD_LIGHT_ONOFF = 0x40
CMD_LIGHT_LEVEL = 0x41
CMD_FAN_ONOFF = 0x42
CMD_FAN_LEVEL = 0x43
CMD_THERMO_TARGET = 0x44
CMD_THERMO_MODE = 0x45
CMD_SOCKET_ONOFF = 0x46
CMD_SCENE = 0x47
CMD_QUERY_ALL = 0x48
CMD_CFG_WRITE = 0x50

DEV_NAMES = {1: "主灯", 2: "灯带", 3: "风扇", 4: "温控", 5: "插座1", 6: "插座2"}
THERMO_MODE_NAMES = {0: "关", 1: "制冷", 2: "制热", 3: "自动"}

CMD_NAMES = {
    CMD_QUERY_STATUS: "查询单设备",
    CMD_HEARTBEAT: "心跳",
    CMD_LIGHT_ONOFF: "灯开关",
    CMD_LIGHT_LEVEL: "灯亮度",
    CMD_FAN_ONOFF: "风扇开关",
    CMD_FAN_LEVEL: "风扇挡位",
    CMD_THERMO_TARGET: "温控目标",
    CMD_THERMO_MODE: "温控模式",
    CMD_SOCKET_ONOFF: "插座开关",
    CMD_SCENE: "场景(TODO-M4)",
    CMD_QUERY_ALL: "查询全部",
    CMD_CFG_WRITE: "配置写入",
}


# ---------------------------------------------------------------------------
# CRC16-CCITT（poly 0x1021 / init 0xFFFF / MSB-first）——与 Rust crc16.rs 同参
# ---------------------------------------------------------------------------

def crc16_ccitt(data: bytes) -> int:
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if (crc & 0x8000) else (crc << 1)
            crc &= 0xFFFF
    return crc


# ---------------------------------------------------------------------------
# 帧编解码
# ---------------------------------------------------------------------------

@dataclass
class Frame:
    frame_type: int
    cmd: int
    seq: int
    sid: int
    payload: bytes

    def encode(self) -> bytes:
        if len(self.payload) > FRAME_PAYLOAD_MAX:
            raise ValueError(f"payload too long: {len(self.payload)}")
        # 手工拼装避免 struct 端序混淆：seq 大端、CRC 小端
        buf = bytearray()
        buf += bytes([FRAME_HEAD1, FRAME_HEAD2, FRAME_VERSION,
                      self.frame_type, self.cmd, len(self.payload)])
        buf += struct.pack(">H", self.seq)
        buf += bytes([self.sid])
        buf += self.payload
        crc = crc16_ccitt(bytes(buf))
        buf += struct.pack("<H", crc)
        return bytes(buf)

    @staticmethod
    def decode(buf: bytes) -> "Frame":
        if len(buf) < FRAME_HEADER_SIZE + 2:
            raise ValueError(f"frame too short: {len(buf)}")
        if buf[0] != FRAME_HEAD1 or buf[1] != FRAME_HEAD2:
            raise ValueError(f"bad magic: {buf[0]:02X} {buf[1]:02X}")
        if buf[2] != FRAME_VERSION:
            raise ValueError(f"bad version: 0x{buf[2]:02X}")
        length = buf[5]
        need = FRAME_HEADER_SIZE + length + 2
        if len(buf) < need:
            raise ValueError(f"len mismatch: declared {length}, got {len(buf)}")
        crc_calc = crc16_ccitt(bytes(buf[:FRAME_HEADER_SIZE + length]))
        crc_rx = struct.unpack("<H", buf[need - 2:need])[0]
        if crc_calc != crc_rx:
            raise ValueError(f"CRC mismatch: rx {crc_rx:04X} calc {crc_calc:04X}")
        return Frame(
            frame_type=buf[3],
            cmd=buf[4],
            seq=struct.unpack(">H", buf[6:8])[0],
            sid=buf[8],
            payload=bytes(buf[FRAME_HEADER_SIZE:FRAME_HEADER_SIZE + length]),
        )


# ---------------------------------------------------------------------------
# seq 去重滑动窗口（逐位复刻 sh_frame_seq_fresh）
# ---------------------------------------------------------------------------

class SeqWindow:
    def __init__(self) -> None:
        self.last = 0
        self.mask = 0

    def fresh(self, seq: int) -> bool:
        diff = (self.last - seq) & 0xFFFF
        if diff < FRAME_SEQ_WINDOW:
            bit = 1 << diff
            if self.mask & bit:
                return False
            self.mask |= bit
            return True
        # 窗口外：视为新流（首次或跳变），重置窗口
        self.last = seq
        self.mask = 1
        return True


# ---------------------------------------------------------------------------
# 设备状态机
# ---------------------------------------------------------------------------

@dataclass
class Device:
    id: int
    onoff: int = 0
    level: int = 0
    temp_x10: int = 0
    err: int = 0
    # 温控专属
    thermo_target_x10: int = 240
    thermo_mode: int = 0

    def status_entry(self) -> bytes:
        return struct.pack("<BBBhbB", self.id, self.onoff, self.level,
                           self.temp_x10, self.err, 0)


@dataclass
class Stats:
    rx_frames: int = 0
    tx_acks: int = 0
    tx_status: int = 0
    bad_frames: int = 0
    dropped_stale: int = 0


class ArtPiSim:
    def __init__(self, room_temp_x10: int = 240) -> None:
        self.devices = {i: Device(id=i) for i in range(1, 7)}
        self.devices[4].temp_x10 = room_temp_x10
        self.seq_win = SeqWindow()
        self.stats = Stats()
        self._temp_frac = 0.0        # 温控步进小数累积器
        self.last_rx_from: tuple | None = None
        self.last_rx_at = 0.0
        self.dirty = False           # 有命令变更 → 即时上报
        self.last_status_at = 0.0
        self.status_period = 5.0

    # ---- 协议处理 ------------------------------------------------------

    def handle(self, buf: bytes, addr: tuple) -> list[bytes]:
        """处理一个 UDP 数据报，返回要回发的帧字节列表。"""
        try:
            f = Frame.decode(buf)
        except ValueError as e:
            self.stats.bad_frames += 1
            log(f"✗ 坏帧丢弃（{len(buf)}B）: {e}")
            return []

        self.stats.rx_frames += 1
        self.last_rx_from = addr
        self.last_rx_at = time.monotonic()

        if not self.seq_win.fresh(f.seq):
            self.stats.dropped_stale += 1
            log(f"↺ seq 重放丢弃（seq={f.seq} cmd=0x{f.cmd:02X} "
                f"{CMD_NAMES.get(f.cmd, '?')}）—— PC 侧重传必须换 seq")
            return []

        return self.execute(f, addr)

    def execute(self, f: Frame, addr: tuple) -> list[bytes]:
        name = CMD_NAMES.get(f.cmd, f"未知0x{f.cmd:02X}")
        out: list[bytes] = []

        if f.cmd == CMD_HEARTBEAT:
            log(f"← 心跳（seq={f.seq} sid={f.sid} from={addr[0]}）")
            out.append(self.ack(f, b""))

        elif f.cmd == CMD_QUERY_STATUS:
            dev = self._dev_from_payload(f.payload, 0)
            log(f"← 查询单设备 {dev and DEV_NAMES.get(dev, dev)}"
                f"（seq={f.seq}）")
            if dev in self.devices:
                out.append(self.ack(f, self.devices[dev].status_entry()))

        elif f.cmd == CMD_QUERY_ALL:
            log(f"← 查询全部（seq={f.seq}）")
            out.append(self.ack(f, self.status_payload()))

        elif f.cmd in (CMD_LIGHT_ONOFF, CMD_LIGHT_LEVEL, CMD_FAN_ONOFF,
                       CMD_FAN_LEVEL, CMD_SOCKET_ONOFF):
            if len(f.payload) < 2:
                log(f"✗ {name} 载荷不足（{f.payload.hex()}）")
                return []
            dev, val = f.payload[0], f.payload[1]
            if dev not in self.devices:
                log(f"✗ {name} 设备不存在: {dev}")
                return []
            d = self.devices[dev]
            if f.cmd in (CMD_LIGHT_ONOFF, CMD_FAN_ONOFF, CMD_SOCKET_ONOFF):
                d.onoff = val
                log(f"⚙ {DEV_NAMES[dev]}{'开' if val else '关'}"
                    f"（seq={f.seq}）")
            else:
                d.level = min(val, 100)
                log(f"⚙ {DEV_NAMES[dev]}{'亮度' if f.cmd == CMD_LIGHT_LEVEL else '挡位'}"
                    f"{d.level}%（seq={f.seq}）")
            self.dirty = True
            out.append(self.ack(f, b""))

        elif f.cmd == CMD_THERMO_TARGET:
            if len(f.payload) < 3:
                log("✗ 温控目标载荷不足")
                return []
            zone = f.payload[0]
            target = struct.unpack("<h", f.payload[1:3])[0]
            if zone not in self.devices:
                log(f"✗ 温控区不存在: {zone}")
                return []
            d = self.devices[zone]
            d.thermo_target_x10 = target
            log(f"⚙ 温控目标 {target / 10:.1f}℃（seq={f.seq}）")
            self.dirty = True
            out.append(self.ack(f, b""))

        elif f.cmd == CMD_THERMO_MODE:
            if len(f.payload) < 2:
                log("✗ 温控模式载荷不足")
                return []
            zone, mode = f.payload[0], f.payload[1]
            if zone not in self.devices:
                log(f"✗ 温控区不存在: {zone}")
                return []
            d = self.devices[zone]
            d.thermo_mode = mode
            log(f"⚙ 温控模式 {THERMO_MODE_NAMES.get(mode, mode)}（seq={f.seq}）")
            self.dirty = True
            out.append(self.ack(f, b""))

        elif f.cmd == CMD_SCENE:
            # ⚠️ 复刻真源：cmd_dispatch.c 的 on_scene 是 TODO(M4) 桩——
            # 收帧回 Ack 但不执行任何设备动作。Companion 因此默认在 PC 侧
            # 把场景展开成 0x40–0x46 设备命令（scene_mode="expand"）。
            scene = f.payload[0] if f.payload else 0
            log(f"⚠ 场景 0x47 id={scene} → TODO(M4) 桩：回 Ack 不执行"
                f"（seq={f.seq}）")
            out.append(self.ack(f, b""))

        elif f.cmd == CMD_CFG_WRITE:
            log(f"← 配置写入 {len(f.payload)}B（seq={f.seq}）")
            out.append(self.ack(f, b""))

        else:
            log(f"← 未知命令 0x{f.cmd:02X}（seq={f.seq}）")
            out.append(self.ack(f, b""))

        return out

    def ack(self, req: Frame, payload: bytes) -> bytes:
        self.stats.tx_acks += 1
        return Frame(FT_ACK, req.cmd, req.seq, req.sid, payload).encode()

    def status_payload(self) -> bytes:
        return b"".join(d.status_entry() for d in self.devices.values())

    def status_frame(self) -> bytes:
        self.stats.tx_status += 1
        # 状态帧 seq 自增（不参与对端去重语义的状态回传通道）
        if not hasattr(self, "_status_seq"):
            self._status_seq = 1
        f = Frame(FT_STATUS, CMD_QUERY_ALL, self._status_seq, 0,
                  self.status_payload())
        self._status_seq = (self._status_seq + 1) & 0xFFFF or 1
        return f.encode()

    # ---- 温控仿真（一阶趋近，真源是迟滞+PID） --------------------------

    def tick_thermo(self, dt: float) -> None:
        d = self.devices[4]
        if d.thermo_mode == 0:  # OFF：不动作
            return
        # 0.5°C/s = 5 个 x10 单位/s。50ms tick 只有 0.25 单位，
        # 必须用小数累积器，否则 int() 截断会把每 tick 抹成 0。
        rate = 5.0 * dt + self._temp_frac
        step = int(rate)
        self._temp_frac = rate - step
        if step == 0:
            return
        if d.thermo_mode == 1:      # 制冷：只降
            d.temp_x10 = max(d.thermo_target_x10, d.temp_x10 - step)
        elif d.thermo_mode == 2:    # 制热：只升
            d.temp_x10 = min(d.thermo_target_x10, d.temp_x10 + step)
        else:                       # 自动：双向趋近
            delta = d.thermo_target_x10 - d.temp_x10
            d.temp_x10 += max(-step, min(step, delta))

    def _dev_from_payload(self, payload: bytes, idx: int) -> int | None:
        return payload[idx] if len(payload) > idx else None


# ---------------------------------------------------------------------------
# 日志
# ---------------------------------------------------------------------------

_START = time.monotonic()
_LOG_FH = None  # --log 指定的日志文件句柄（未指定则只打屏幕）


def set_log_file(path: str) -> None:
    """打开日志文件。

    屏幕上的日志会随终端关闭而消失，但演示视频重录、作业文档贴证据
    都需要可归档的文本，所以每行同时写一份到文件。
    """
    global _LOG_FH
    _LOG_FH = open(path, "a", encoding="utf-8", buffering=1)


def log(msg: str) -> None:
    t = time.monotonic() - _START
    line = f"[{t:7.1f}s] {msg}"
    print(line, flush=True)
    if _LOG_FH is not None:
        _LOG_FH.write(line + "\n")


# ---------------------------------------------------------------------------
# 主循环
# ---------------------------------------------------------------------------

def serve(port: int, room_temp_x10: int) -> None:
    sim = ArtPiSim(room_temp_x10)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("0.0.0.0", port))
    sock.settimeout(0.05)

    log("=" * 62)
    log("ART-Pi V2.3 协议模拟器（复刻 ctrl_server/frame_codec 行为）")
    log(f"监听 0.0.0.0:{port} · 室温 {room_temp_x10 / 10:.1f}℃ · "
        f"状态周期 {sim.status_period:.0f}s")
    log("场景 0x47 = TODO(M4) 桩（与真源一致）：Companion 须用 expand 模式")
    log("=" * 62)

    last_tick = time.monotonic()
    try:
        while True:
            # 1) 收包（50ms 超时轮询）
            try:
                buf, addr = sock.recvfrom(FRAME_MAX)
                for reply in sim.handle(buf, addr):
                    sock.sendto(reply, addr)
            except socket.timeout:
                pass
            except (ConnectionResetError, OSError) as e:
                # Windows UDP 语义：状态帧打到一个已关闭的对端端口时，
                # ICMP 不可达会回灌到 recvfrom（10054）——视为该对端消失，
                # 清空记录，停止周期发送，等新客户端帧到来再更新。
                if isinstance(e, ConnectionResetError) and sim.last_rx_from:
                    log(f"⚠ 对端 {sim.last_rx_from} 不可达（10054），"
                        f"暂停状态帧，等待新客户端")
                    sim.last_rx_from = None
                else:
                    log(f"⚠ 接收异常已忽略: {e!r}")

            # 2) 温控步进
            now = time.monotonic()
            sim.tick_thermo(now - last_tick)
            last_tick = now

            # 3) 状态帧：命令变更即时 + 周期 5s
            if sim.dirty or (now - sim.last_status_at >= sim.status_period):
                if sim.last_rx_from is not None:
                    sock.sendto(sim.status_frame(), sim.last_rx_from)
                sim.dirty = False
                sim.last_status_at = now

    except KeyboardInterrupt:
        log("-" * 62)
        log(f"收帧 {sim.stats.rx_frames} · Ack {sim.stats.tx_acks} · "
            f"状态帧 {sim.stats.tx_status} · "
            f"坏帧 {sim.stats.bad_frames} · seq重放丢弃 {sim.stats.dropped_stale}")
        log("模拟器退出")


def main() -> None:
    ap = argparse.ArgumentParser(description="ART-Pi V2.3 protocol simulator")
    ap.add_argument("--port", type=int, default=17335)
    ap.add_argument("--room-temp", type=int, default=240,
                    help="initial room temperature x10 (240 = 24.0C)")
    ap.add_argument("--log", default=None,
                    help="同时把日志写入该文件（追加模式），用于留证")
    args = ap.parse_args()

    if args.log:
        set_log_file(args.log)
        log(f"日志文件: {args.log}")
    serve(args.port, args.room_temp)


if __name__ == "__main__":
    main()
