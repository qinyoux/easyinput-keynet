#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""ART-Pi 协议级探测器：找到网段里真正的 ART-Pi 板子。

**为什么不用 MAC 判断**：板子连的是 vinno-guest（DHCP，IP 不固定），
AP6212 的 MAC 前缀与同网段其他设备难以可靠区分。真正可靠的判据是
**协议行为**：发一帧合法的 V2.3 心跳，只有跑着 ctrl_server.c 的板子
会回 Ack。

帧格式与两侧实现逐字段对齐：
- ART-Pi  `sh_frame.h`：0x5A 0xA5 + ver 0x02 + type + cmd + len + seq(大端) + sid + payload + CRC16 小端
- Companion `v23/frame.rs`：同格式
所以本探测器发的帧，真机与模拟器都认。

用法：
    python artpi_probe.py                    # 扫 ARP 表里的同网设备
    python artpi_probe.py --ip 10.10.20.177  # 只测指定 IP
    python artpi_probe.py --range 10.10.20.1-254   # 全网段扫描
"""

from __future__ import annotations

import argparse
import socket
import struct
import subprocess
import sys
import time

# ---- V2.3 协议常量（与 sh_frame.h / companion constants.rs 一致） ----
FRAME_HEAD1 = 0x5A
FRAME_HEAD2 = 0xA5
FRAME_VERSION = 0x02
FT_CMD = 1
FT_ACK = 3
CMD_HEARTBEAT = 0x30
CMD_QUERY_ALL = 0x48
CMD_QUERY_STATUS = 0x20
# 家居段（sh_frame.h 0x40-0x5F）——真机联调用
CMD_LIGHT_ONOFF = 0x40
CMD_LIGHT_LEVEL = 0x41
CMD_FAN_ONOFF = 0x42
CMD_FAN_LEVEL = 0x43
CMD_THERMO_TARGET = 0x44
CMD_THERMO_MODE = 0x45
CMD_SOCKET_ONOFF = 0x46
CMD_SCENE = 0x47
CTRL_UDP_PORT = 17335


def crc16_ccitt(data: bytes) -> int:
    """CRC16-CCITT（poly 0x1021，初值 0xFFFF）——与 frame_codec.c 的查表法同结果。"""
    crc = 0xFFFF
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if (crc & 0x8000) else ((crc << 1) & 0xFFFF)
    return crc


def build_frame(cmd: int, seq: int, payload: bytes = b"", frame_type: int = FT_CMD) -> bytes:
    buf = bytearray()
    buf += bytes([FRAME_HEAD1, FRAME_HEAD2, FRAME_VERSION, frame_type, cmd, len(payload)])
    buf += struct.pack(">H", seq)
    buf += bytes([0])                      # sid
    buf += payload
    buf += struct.pack("<H", crc16_ccitt(bytes(buf)))   # CRC16 小端
    return bytes(buf)


def parse_frame(buf: bytes):
    """解出 (type, cmd, seq)。长度或 CRC 不符返回 None。"""
    if len(buf) < 11:
        return None
    if buf[0] != FRAME_HEAD1 or buf[1] != FRAME_HEAD2:
        return None
    if buf[2] != FRAME_VERSION:
        return None
    length = buf[5]
    need = 9 + length + 2
    if len(buf) < need:
        return None
    if crc16_ccitt(bytes(buf[:9 + length])) != struct.unpack("<H", buf[need - 2:need])[0]:
        return None
    return (buf[3], buf[4], struct.unpack(">H", buf[6:8])[0], bytes(buf[9:9 + length]))


def arp_hosts(prefix: str) -> list[str]:
    """从 ARP 表取同网段主机（PC 与板子互通过程中 ARP 会留下条目）。

    ⚠️ Windows `arp -a` 输出是 **GBK** 编码，直接按 UTF-8 解会抛
    UnicodeDecodeError（中文字段"动态"占 2 字节且落在非 ASCII 区）。
    """
    out: list[str] = []
    try:
        r = subprocess.run(["arp", "-a"], capture_output=True, timeout=5)
        txt = r.stdout.decode("gbk", errors="replace")
    except Exception:
        return out
    for line in txt.splitlines():
        line = line.strip()
        if not line.startswith(prefix + "."):
            continue
        # 只取"动态/Static"类型条目——static 多为网关/组播，不会是板子
        low = line.lower()
        if "动态" in line or "dynamic" in low:
            out.append(line.split()[0])
    return sorted(set(out))


def probe(ip: str, timeout: float = 0.6) -> dict | None:
    """发心跳帧；回 Ack 则确认为 ART-Pi。返回探测详情（含状态载荷）。"""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(timeout)
    try:
        seq = 0x1000
        sock.sendto(build_frame(CMD_HEARTBEAT, seq), (ip, CTRL_UDP_PORT))
        ack = None
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                buf, _ = sock.recvfrom(256)
            except socket.timeout:
                continue
            f = parse_frame(buf)
            if f and f[0] == FT_ACK and f[2] == seq:
                ack = f
                break
        if ack is None:
            return None
        # 已确认存活，再查一次全设备状态（0x48 的 Ack 载荷 = 6×7B）
        seq2 = 0x1001
        sock.sendto(build_frame(CMD_QUERY_ALL, seq2), (ip, CTRL_UDP_PORT))
        status = None
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                buf, _ = sock.recvfrom(256)
            except socket.timeout:
                continue
            f = parse_frame(buf)
            if f and f[0] == FT_ACK and f[2] == seq2:
                status = f[3]
                break
        return {"ip": ip, "ack_seq": ack[2], "status": status}
    finally:
        sock.close()


def fmt_status(payload: bytes | None) -> str:
    if not payload or len(payload) % 7:
        return "（无状态载荷）"
    parts = []
    for i in range(0, len(payload) - 6, 7):
        e = payload[i:i + 7]
        temp = struct.unpack("<h", e[3:5])[0]
        s = f"#{e[0]} {'开' if e[1] else '关'}"
        if e[2]:
            s += f"·{e[2]}%"
        if temp:
            s += f"·{temp / 10:.1f}℃"
        parts.append(s)
    return " ".join(parts)


def main() -> int:
    global CTRL_UDP_PORT
    ap = argparse.ArgumentParser()
    ap.add_argument("--ip", default=None, help="只测这个 IP")
    ap.add_argument("--range", dest="rng", default=None,
                    help="全网段扫描，如 10.10.20.1-254")
    ap.add_argument("--port", type=int, default=CTRL_UDP_PORT)
    args = ap.parse_args()

    if args.port:
        CTRL_UDP_PORT = args.port

    if args.ip:
        hosts = [args.ip]
    elif args.rng:
        lo, hi = args.rng.rsplit(".", 1)[1].split("-")
        base = args.rng.rsplit(".", 1)[0]
        hosts = [f"{base}.{i}" for i in range(int(lo), int(hi) + 1)]
    else:
        mine = socket.gethostbyname(socket.gethostname())
        prefix = ".".join(mine.split(".")[:3])
        print(f"本机 {mine} → 扫 ARP 表中 {prefix}.x 的条目")
        hosts = arp_hosts(prefix)
        if not hosts:
            print("ARP 表里没有同网条目，改用 --range 全网段扫描")
            return 1
        print(f"候选 {len(hosts)} 个: {', '.join(hosts)}")

    print(f"\n逐个发 0x30 心跳（UDP :{CTRL_UDP_PORT}），回 Ack 者即 ART-Pi\n")
    found = []
    for ip in hosts:
        try:
            r = probe(ip)
        except OSError:
            continue
        if r:
            print(f"  ✅ {ip}  →  Ack(seq={r['ack_seq']:#06x})  {fmt_status(r['status'])}")
            found.append(r)
        else:
            print(f"  ·  {ip}  无响应")

    print()
    if not found:
        print("[结论] 没找到 ART-Pi。查：① 板子是否连上 vinno-guest（FinSH ifconfig）"
              "\n       ② 是否与 PC 同 AP 同网段  ③ 防火墙是否拦了 UDP 入站")
        return 1
    print(f"[结论] 找到 {len(found)} 个 ART-Pi：{', '.join(f['ip'] for f in found)}")
    print("\n下一步：Companion 配置页把 ART-Pi IP 填成上面的地址，或命令行跑")
    print(f"  cd companion/src-tauri && cargo run --bin scene-walk -- {found[0]['ip']} 17335")
    return 0


if __name__ == "__main__":
    sys.exit(main())
