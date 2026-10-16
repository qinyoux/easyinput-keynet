#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""ART-Pi 模拟器集成测试客户端。

扮演 PC Companion 的 UdpLink：发命令 → 等 Ack → 200ms 未确认则**换新 seq**
重传（≤3 次）→ 解析 7B 状态条目。覆盖协议集成层用例，其中一组专门钉死
"最隐蔽的坑"（ART-Pi 16 槽 seq 去重窗口会吃掉同 seq 重传；2026-10-16
起另含递增流重放回归 + 旧算法负面实证，与 host_test 同源）。

前置：先运行 `python artpi_sim.py`（另开终端）。

用法：python test_client.py [--port 17335]
"""

from __future__ import annotations

import argparse
import inspect
import queue
import socket
import struct
import sys
import threading
import time

from artpi_sim import (CMD_HEARTBEAT, CMD_LIGHT_LEVEL, CMD_LIGHT_ONOFF,
                       CMD_QUERY_ALL, CMD_QUERY_STATUS, CMD_SCENE,
                       CMD_THERMO_MODE, CMD_THERMO_TARGET, FT_ACK,
                       FT_STATUS, FRAME_MAX, FRAME_SEQ_WINDOW,
                       STATUS_ENTRY_SIZE, Frame, SeqWindow, crc16_ccitt)

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

    def drain(self) -> None:
        """清空接收队列里已到达但还没读的帧。

        状态帧周期 5s，而命令变更会触发**即时**上报——所以"发命令 → 2s 后读"
        这段时间里，队列里可能已经躺着一帧**命令发出之前**生成的旧状态帧。
        `wait_status` 无条件返回第一帧，读到的就是它⇒ 断言拿到的是过期数据
        （2026-10-15 实测：温控用例读到 200 而非上升后的值）。
        每个需要"命令生效后的状态"的断言前，先 drain 掉存量。
        """
        while True:
            try:
                self.rx.get_nowait()
            except queue.Empty:
                return

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

    def temp_now(self, timeout: float = 1.0):
        """**同步**读一次温控当前温度（0x43 查询单设备的 Ack 直接带 7B 状态条目）。

        不用状态帧来读温度，是因为状态帧有两套节拍：命令变更**即时**上报
        （`dirty`）与 5s **周期**上报，且命令若落在周期边界附近，下一帧要等
        满5s。任何"sleep N秒 再读帧"的写法都是不确定的——2026-10-15 实测
        温控用例恒读到 `→ 200`：等2s 没等到新帧就超时，返回了队列里的旧帧。
        查询命令是请求-应答，读到的必然是服务端当下的值。
        """
        seq = self.send(CMD_QUERY_STATUS, bytes([4]))
        ack = self.wait_ack(seq, timeout=timeout)
        if ack is None or len(ack.payload) < STATUS_ENTRY_SIZE:
            return None
        return struct.unpack("<h", ack.payload[3:5])[0]

    def dev_state(self, dev_id: int, timeout: float = 1.0):
        """同步查询单设备状态（0x20 请求-应答）：(onoff, level, temp_x10)。

        与 temp_now 同一理由：读"命令生效后的值"必须走查询，不能
        sleep+读帧（状态帧两套节拍会还你旧帧）。
        """
        seq = self.send(CMD_QUERY_STATUS, bytes([dev_id]))
        ack = self.wait_ack(seq, timeout=timeout)
        if ack is None or len(ack.payload) < STATUS_ENTRY_SIZE:
            return None
        p = ack.payload
        return (p[1], p[2], struct.unpack("<h", p[3:5])[0])

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


class OldSeqWindow:
    """修复前的旧算法复刻——**只用于负面测试**，证明新回归用例咬得住。

    对应 frame_codec.c 2026-10-16 前的实现：`diff>=16 一律重置窗口`。
    递增流下每帧重置、位图永远只剩 1 位 ⇒ last-1 重放恒被放行。
    """

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
        self.last = seq
        self.mask = 1
        return True


def _incr_stream_replay_eaten(win_cls) -> bool:
    """递增流 10→11→12→13 后重放 12：修复后必须被吃。"""
    w = win_cls()
    for s in (10, 11, 12, 13):
        w.fresh(s)
    return w.fresh(12) is False


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

    # 2026-10-16 seq 去重修复回归（与 host_test/test_frame_codec.c 同源用例，
    # 固件侧实测背景：丢包重传同 seq 会被二次执行）。旧算法下这些用例**全部
    # 失败**——文末负面实证钉死这一点，防"静默通过"。
    w = SeqWindow()
    for s in (10, 11, 12, 13):
        w.fresh(s)
    check("seq 窗口: 递增流重放被吃", w.fresh(12) is False,
          "旧算法行为：位图被每帧重置，重放被放行")
    check("seq 窗口: 递增流同 seq 被吃", w.fresh(13) is False)
    check("seq 窗口: 递增流新 seq 放行", w.fresh(14))
    w = SeqWindow()
    w.fresh(65534)
    w.fresh(65535)
    check("seq 窗口: 回绕 65535→0 放行", w.fresh(0))
    check("seq 窗口: 回绕后 65535 重放被吃", w.fresh(65535) is False)
    w = SeqWindow()
    w.fresh(100)
    check("seq 窗口: 大跳变按新流放行", w.fresh(40000))
    check("seq 窗口: 新流同 seq 被吃", w.fresh(40000) is False)
    w = SeqWindow()
    w.fresh(7)
    w.reset()
    check("seq 窗口: reset 后同 seq 放行", w.fresh(7))
    check("负面实证: 旧算法放行递增流重放（证明上面咬得住）",
          not _incr_stream_replay_eaten(OldSeqWindow),
          "旧算法也拦住了？回归用例是摆设")

    f = Frame(1, CMD_LIGHT_ONOFF, 0x0102, 7, bytes([1, 1]))
    check("帧 roundtrip", Frame.decode(f.encode()) == f)

    # 钉死"读温度必须走同步查询"这条判据。曾经的温控用例用
    # 「sleep 2s → wait_status 读帧」，而状态帧有 5s 周期，命令落在周期
    # 边界附近时读到的必然是**旧帧**——间歇性假失败，根因极难定位。
    # temp_now 走 0x20 请求-应答，读到的永远是服务端当下值。
    check("temp_now 读的是同步查询而非状态帧",
          inspect.getsource(Client.temp_now).count("CMD_QUERY_STATUS") == 1,
          "若被改回读状态帧，此断言立即失败")

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

        print("== 场景 0x47（本地展开，2026-10-16 起真实现） ==")
        # 2026-10-16 前这里是 TODO(M4) 桩用例（只回 Ack 不执行）——
        # 真源 sh_scene.c 落地后，必须验证「回 Ack **且**设备状态真的变」。
        # 状态读值走 0x20 同步查询（禁 sleep+读帧，理由见 dev_state 注释）。
        ack = c.send_reliable(CMD_SCENE, bytes([4]))   # 影院：主灯30% + 插座1开
        check("0x47 → Ack", ack is not None)
        st = c.dev_state(1)
        check("场景「影院」主灯亮度 30%", st is not None and st[1] == 30,
              f"实际 {st}")
        st = c.dev_state(5)
        check("场景「影院」插座1 开", st is not None and st[0] == 1,
              f"实际 {st}")
        ack = c.send_reliable(CMD_SCENE, bytes([2]))   # 离家：全关 + 温控关
        check("0x47 离家 → Ack", ack is not None)
        st = c.dev_state(1)
        check("场景「离家」主灯关", st is not None and st[0] == 0,
              f"实际 {st}")
        ack = c.send_reliable(CMD_SCENE, bytes([9]))   # 未知场景 id
        check("0x47 未知 id → 仍回 Ack（复刻真源失败也 Ack）", ack is not None)

        print("== 温控仿真 ==")
        # ⚠️ 本用例踩过三个坑，每一个都表现为"报失败但产品代码完全正常"：
        #
        # 1. **依赖初始状态**：先用 scene-walk 把温控推到 22.0℃ 制冷，室温
        #    已到 22.0℃；本用例再设 20.0℃ 制冷，室温已在下降通道端点
        #    ⇒ `t1 < t0` 必然不成立。
        # 2. **依赖状态帧时序**：状态帧有"变更即时"与"5s 周期"两套节拍，
        #    命令落在周期边界附近时下一帧要等满 5s。原写法 sleep 2s 再读帧，
        #    恒定超时并返回队列里的**旧帧**（实测恒为 `→ 200`）。
        #    ⇒ 改用 0x20 查询单设备：请求-应答，读到的必然是当下值。
        # 3. **固定等待不足以跨过距离**：步进 0.5℃/s（5 个 x10/s）。上一轮
        #    若把温度停在 20.0℃，爬到 28℃ 需 16s，而 sleep 2s 只走 10 个单位
        #    ⇒ 起点原地不动，制冷段自然 `200 → 200`（实测间歇失败）。
        #    ⇒ 改为**轮询到温度真正离开起点**再切方向，不赌固定时长。
        #
        # 方向单调性由模拟器的 max/min 钳位保证：制热只升、制冷只降，
        # 所以"离开起点"必然发生，只可能慢，不会反向。
        def wait_temp_change(client, baseline, timeout=6.0, interval=0.3):
            """轮询直到温度偏离 baseline；返回 (新温度, 是否变化)。"""
            deadline = time.monotonic() + timeout
            t = baseline
            while time.monotonic() < deadline:
                time.sleep(interval)
                t = client.temp_now()
                if t is not None and t != baseline:
                    return t, True
            return t, False

        # --- 制热：先把温度抬离下限（目标远高于任何可能的起点） ---
        c.send_reliable(CMD_THERMO_MODE, bytes([4, 2]))  # 制热
        c.send_reliable(CMD_THERMO_TARGET, bytes([4]) + struct.pack("<h", 400))
        base_hot = c.temp_now()
        t_heat1, heated = wait_temp_change(c, base_hot)
        check("温控制热温度上升", heated,
              f"{base_hot} → {t_heat1}（制热应单调上升）")

        # --- 制冷：目标远低于刚才的制热温度，必降 ---
        c.send_reliable(CMD_THERMO_TARGET, bytes([4]) + struct.pack("<h", 100))
        c.send_reliable(CMD_THERMO_MODE, bytes([4, 1]))  # 制冷
        base_cool = c.temp_now()
        t_cool1, cooled = wait_temp_change(c, base_cool)
        check("温控制冷温度下降", cooled,
              f"{base_cool} → {t_cool1}（制冷应单调下降）")
    finally:
        c.close()

    print(f"\n结果: {PASS} 通过 / {FAIL} 失败")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
