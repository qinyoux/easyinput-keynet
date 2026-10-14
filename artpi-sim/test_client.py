#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""ART-Pi 模拟器集成测试客户端。

扮演 PC Companion 的 UdpLink：发命令 → 等 Ack → 200ms 未确认则**换新 seq**
重传（≤3 次）→ 解析 7B 状态条目。覆盖协议集成层用例，其中两条专门钉死
"最隐蔽的坑"（ART-Pi 16 槽 seq 去重窗口会吃掉同 seq 重传）。

前置：先运行 `python artpi_sim.py`（另开终端）。

用法：python test_client.py [--port 17335]
"""

from __future__ import annotations

import argparse
import queue
import socket
import struct
import sys
import threading
import time

from artpi_sim import (CMD_HEARTBEAT, CMD_LIGHT_LEVEL, CMD_LIGHT_ONOFF,
                       CMD_QUERY_ALL, CMD_QUERY_STATUS, CMD_SCENE,
                       CMD_THERMO_MODE, CMD_THERMO_TARGET, FT_ACK,
                       FT_STATUS, FRAME_MAX, STATUS_ENTRY_SIZE, Frame,
                       SeqWindow, crc16_ccitt)

PASS = 0
FAIL = 0


def check(name: str, cond: bool, detail: str = "") -> None:
    global PASS, FAIL
    if cond:
        PASS += 1
        print(f"  ✓ {name}")
    else:
        FAIL += 1
        print(f"  ✗ {name}  {detail}")


class Client:
    """复刻 Companion UdpLink 的重传策略：换 seq 才重发。"""

    def __init__(self, port: int) -> None:
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.05)
        self.addr = ("127.0.0.1", port)
        self.seq = 1
        self.rx: "queue.Queue[Frame]" = queue.Queue()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._rx_loop, daemon=True)
        self.thread.start()

    def _rx_loop(self) -> None:
        while not self.stop.is_set():
            try:
                buf, _ = self.sock.recvfrom(FRAME_MAX)
                self.rx.put(Frame.decode(buf))
            except socket.timeout:
                continue
            except (ConnectionResetError, OSError):
                # Windows UDP 语义：曾向不可达端口发包后，recvfrom 会报
                # 10054（ConnectionResetError）——只影响当次调用，socket
                # 仍可用，忽略即可。
                continue

    def next_seq(self) -> int:
        s = self.seq
        self.seq = (self.seq + 1) & 0xFFFF or 1
        return s

    def send(self, cmd: int, payload: bytes = b"", seq: int | None = None):
        s = self.next_seq() if seq is None else seq
        self.sock.sendto(Frame(1, cmd, s, 0, payload).encode(), self.addr)
        return s

    def send_raw(self, data: bytes) -> None:
        self.sock.sendto(data, self.addr)

    def wait_ack(self, seq: int, timeout: float = 1.0):
        """等指定 seq 的 Ack；顺带收集状态帧。"""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                f = self.rx.get(timeout=0.05)
            except queue.Empty:
                continue
            if f.frame_type == FT_ACK and f.seq == seq:
                return f
        return None

    def send_reliable(self, cmd: int, payload: bytes = b"", label: str = ""):
        """首送 + 200ms 换 seq 重传 ≤3 次（与 RetransmitQueue 同参）。"""
        seq = self.send(cmd, payload)
        for _ in range(3):
            ack = self.wait_ack(seq, timeout=0.2)
            if ack is not None:
                return ack
            seq = self.send(cmd, payload)  # 换新 seq 重发
        return self.wait_ack(seq, timeout=0.2)

    def wait_status(self, timeout: float = 6.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                f = self.rx.get(timeout=0.1)
            except queue.Empty:
                continue
            if f.frame_type == FT_STATUS:
                return f
        return None

    def close(self) -> None:
        self.stop.set()
        self.thread.join(timeout=0.5)
        self.sock.close()


def parse_states(payload: bytes):
    out = []
    for i in range(0, len(payload) - STATUS_ENTRY_SIZE + 1, STATUS_ENTRY_SIZE):
        e = payload[i:i + STATUS_ENTRY_SIZE]
        out.append({"id": e[0], "onoff": e[1], "level": e[2],
                    "temp_x10": struct.unpack("<h", e[3:5])[0], "err": e[5]})
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=17335)
    args = ap.parse_args()

    print("== 协议金标准 ==")
    check("crc16_ccitt('123456789') == 0x29B1",
          crc16_ccitt(b"123456789") == 0x29B1,
          f"got {crc16_ccitt(b'123456789'):04X}")

    w = SeqWindow()
    check("seq 窗口: 同 seq 丢弃", (w.fresh(42), w.fresh(42)) == (True, False))
    check("seq 窗口: 换 seq 放行", w.fresh(43))

    f = Frame(1, CMD_LIGHT_ONOFF, 0x0102, 7, bytes([1, 1]))
    check("帧 roundtrip", Frame.decode(f.encode()) == f)

    print("== 连接模拟器 ==")
    c = Client(args.port)
    try:
        # 心跳唤醒链路（模拟器记录对端地址后才回状态帧）
        ack = c.send_reliable(CMD_HEARTBEAT, label="heartbeat")
        check("心跳 → 空 Ack", ack is not None and ack.payload == b"",
              f"ack={ack}")

        print("== 设备命令 ==")
        ack = c.send_reliable(CMD_LIGHT_ONOFF, bytes([1, 1]))
        check("主灯开 → Ack", ack is not None)

        ack = c.send_reliable(CMD_LIGHT_LEVEL, bytes([1, 80]))
        check("主灯 80% → Ack", ack is not None)

        st = c.wait_status()
        check("收到状态帧（变化即时）", st is not None)
        if st:
            states = {s["id"]: s for s in parse_states(st.payload)}
            check("状态含 6 设备", len(states) == 6, f"got {len(states)}")
            check("主灯状态 = 开/80%",
                  states[1]["onoff"] == 1 and states[1]["level"] == 80,
                  f"got {states[1]}")

        print("== 查询 ==")
        seq = c.send(CMD_QUERY_ALL)
        ack = c.wait_ack(seq)
        check("0x48 查询全部 → Ack 42B",
              ack is not None and len(ack.payload) == 6 * 7,
              f"payload={len(ack.payload) if ack else None}B")

        seq = c.send(CMD_QUERY_STATUS, bytes([4]))
        ack = c.wait_ack(seq)
        check("0x20 查询单设备 → Ack 7B",
              ack is not None and len(ack.payload) == 7)

        print("== 最隐蔽的坑：seq 去重 ==")
        # 先用 0x0ABC 正常发一次并确认链路可达（否则下面的"被丢弃"
        # 会与"链路不通"混淆，出现假通过）。
        c.send(CMD_LIGHT_LEVEL, bytes([1, 55]), seq=0x0ABC)
        first = c.wait_ack(0x0ABC, timeout=0.5)
        check("链路健康：指定 seq 能收到 Ack", first is not None)
        # 同 seq 重放 → 模拟器丢弃 → 永远等不到 Ack
        c.send(CMD_LIGHT_LEVEL, bytes([1, 55]), seq=0x0ABC)
        ack = c.wait_ack(0x0ABC, timeout=0.4)
        check("同 seq 重放被 ART-Pi 丢弃（复现坑）", ack is None,
              "竟然收到了 Ack？！")
        # 换新 seq → 必达
        fresh = c.send(CMD_LIGHT_LEVEL, bytes([1, 55]))
        ack = c.wait_ack(fresh)
        check("换新 seq 送达", ack is not None)

        print("== 健壮性 ==")
        bad = bytearray(Frame(1, CMD_LIGHT_ONOFF, 999, 0, bytes([1, 1])).encode())
        bad[-1] ^= 0xFF
        c.send_raw(bytes(bad))
        time.sleep(0.1)
        ack = c.send_reliable(CMD_HEARTBEAT)
        check("坏 CRC 帧不击垮服务端", ack is not None)

        print("== 场景 0x47（TODO M4 桩） ==")
        ack = c.send_reliable(CMD_SCENE, bytes([1]))
        check("0x47 → Ack（但不执行，复刻真源桩）", ack is not None)

        print("== 温控仿真 ==")
        c.send_reliable(CMD_THERMO_TARGET, bytes([4]) + struct.pack("<h", 200))
        c.send_reliable(CMD_THERMO_MODE, bytes([4, 1]))  # 制冷
        st = c.wait_status()
        t0 = None
        if st:
            t0 = {s["id"]: s for s in parse_states(st.payload)}[4]["temp_x10"]
        time.sleep(2.0)
        st2 = c.wait_status()
        t1 = None
        if st2:
            t1 = {s["id"]: s for s in parse_states(st2.payload)}[4]["temp_x10"]
        check("温控制冷温度下降", t0 is not None and t1 is not None and t1 < t0,
              f"{t0} → {t1}")
    finally:
        c.close()

    print(f"\n结果: {PASS} 通过 / {FAIL} 失败")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
