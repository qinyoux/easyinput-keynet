#!/usr/bin/env python3
"""scene_parity.py — 三端场景一致性校验（ART-Pi C 表 × PC Companion Rust 表 × 模拟器 Python 表）

为什么存在：2026-10-16 ART-Pi 落地 8 场景本地展开（sh_scene.c），场景定义
与 Companion `home/scene.rs::default_scenes()` 逐条对齐，模拟器
`artpi-sim/artpi_sim.py` 的 SCENES 同步复刻。手抄必然漂移，
本脚本把三边的表各自解析成 (cmd, payload) 序列后按场景 id 比对，任何一条
不一致即 exit 1——改一端忘另一端时立刻发现。

用法：python scene_parity.py [--artpi <sh_scene.c>] [--rust <scene.rs>]
                                [--sim <artpi_sim.py>]
      不带参数时用默认相对路径（本脚本位于 companion/tools/）。
"""
import argparse
import re
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve()
REPO = HERE.parents[1]                      # easyinput-keynet/companion/
ART_PI_SCENE = (REPO.parent / "MyART-PI" / "applications" / "src" / "sh_scene.c")
# 注意：MyART-PI 工程在 D:/RT-ThreadStudio/workspace，不在本仓库内。
# 默认路径指向工作区克隆；找不到时用 --artpi 显式指定。
ART_PI_SCENE_ALT = Path("D:/RT-ThreadStudio/workspace/MyART-PI/applications/src/sh_scene.c")
RUST_SCENE = REPO / "src-tauri" / "src" / "home" / "scene.rs"
SIM_SCENE = REPO.parent / "artpi-sim" / "artpi_sim.py"
ART_PI_FRAME_H = (REPO.parent / "MyART-PI" / "applications" / "inc" / "sh_frame.h")
ART_PI_FRAME_H_ALT = Path("D:/RT-ThreadStudio/workspace/MyART-PI/applications/inc/sh_frame.h")
ART_PI_CONFIG_H_ALT = Path("D:/RT-ThreadStudio/workspace/MyART-PI/applications/inc/sh_config.h")
RUST_CONSTANTS = REPO / "src-tauri" / "src" / "constants.rs"


def _first_existing(*paths: Path) -> Path:
    for p in paths:
        if p.exists():
            return p
    raise SystemExit(f"找不到文件（试过：{', '.join(str(p) for p in paths)}）——用参数显式指定")


def parse_defines(path: Path) -> dict:
    """解析 C 头文件里的 #define NAME value（支持 0x 十六进制）。"""
    out = {}
    for m in re.finditer(r"^\s*#define\s+(\w+)\s+(0x[0-9A-Fa-f]+|\d+)u?\b",
                         path.read_text(encoding="utf-8"), re.M):
        out[m.group(1)] = int(m.group(2), 0)
    return out


def parse_enum(path: Path, enum_name: str) -> dict:
    """解析 C 枚举（如 sh_types.h 的 sh_thermo_mode_t），返回 {成员名: 值}。
    主体用否定前瞻挡住 'typedef'，防止非贪婪匹配跨到前一个 enum。"""
    text = path.read_text(encoding="utf-8")
    m = re.search(rf"typedef\s+enum\s*\{{((?:(?!typedef).)*?)\}}\s*{enum_name}\s*;",
                  text, re.S)
    if not m:
        return {}
    out, auto = {}, 0
    for item in m.group(1).split(","):
        item = item.strip()
        em = re.match(r"(\w+)\s*(?:=\s*([^,]+))?$", item)
        if not em:
            continue
        if em.group(2):
            auto = eval(em.group(2), {"__builtins__": {}}, {})  # noqa: S307 受信头文件
        out[em.group(1)] = auto
        auto += 1
    return out


def eval_int(expr: str, defs: dict) -> int:
    """求值 C 侧小表达式：先替换 SH_DEV_ID_*/SH_THERMO_MODE_* 等符号，
    再求值（只允许 数字/空白/&/>> 组合，如 240 & 0xFF）。"""
    expr = expr.strip()
    for name, val in defs.items():
        expr = re.sub(rf"\b{name}\b", str(val), expr)
    if not re.fullmatch(r"[0-9A-Fa-fx\s&>]+", expr):
        raise ValueError(f"非预期表达式: {expr!r}")
    return int(eval(expr, {"__builtins__": {}}, {}))  # noqa: S307 字符集已白名单


def parse_c_scenes(text: str, frame_defs: dict, cfg_defs: dict) -> dict:
    """解析 sh_scene.c：返回 {scene_id: (tag, name, [(cmd, payload), ...])}"""
    # 命令数组块：static const sh_scene_cmd_t c_home[] = { ... };
    blocks = {}
    for m in re.finditer(
            r"static const sh_scene_cmd_t (\w+)\[\]\s*=\s*\{(.*?)\};",
            text, re.S):
        name, body = m.group(1), m.group(2)
        cmds = []
        for cm in re.finditer(
                r"\{\s*(SH_CMD_\w+)\s*,\s*(\d+)\s*,\s*\{([^}]*)\}\s*\}", body):
            cmd = frame_defs[cm.group(1)]
            plen = int(cm.group(2))
            # C 表初始化器常带占位 pad（len=2 却写 3 个），按声明长度截取
            payload = [eval_int(x, {**frame_defs, **cfg_defs})
                       for x in cm.group(3).split(",") if x.strip()][:plen]
            if len(payload) != plen:
                raise ValueError(f"{name}: 声明 len={plen} 实际只有 {len(payload)}")
            cmds.append((cmd, payload))
        blocks[name] = cmds

    # 场景表：{ 1u, "Home", "回家", c_home,   N },
    scenes = {}
    for m in re.finditer(
            r"\{\s*(\d+)u\s*,\s*\"(\w+)\"\s*,\s*\"([^\"]+)\"\s*,\s*(\w+)\s*,"
            r"\s*\(rt_uint8_t\)\(sizeof\(\4\)\s*/\s*sizeof\(\4\[0\]\)\)\s*\}",
            text):
        sid, tag, name, blk = int(m.group(1)), m.group(2), m.group(3), m.group(4)
        scenes[sid] = (tag, name, blocks[blk])
    return scenes


def parse_rust_scenes(text: str, rust_consts: dict) -> dict:
    """解析 scene.rs default_scenes()：返回 {scene_id: (name, [(cmd, payload), ...])}"""
    body = text[text.index("pub fn default_scenes()"):]
    body = body[:body.index("\n}\n")] if "\n}\n" in body else body

    def dev(v: str) -> int:
        if v.startswith("DEV_ID_"):
            return rust_consts[v]
        return int(v)

    scenes = {}
    for sm in re.finditer(
            r"SceneDef\s*\{\s*id:\s*(\d+)\s*,\s*name:\s*\"([^\"]+)\"\s*\.into\(\),\s*commands:\s*vec!\[(.*?)\],\s*\}",
            body, re.S):
        sid, name = int(sm.group(1)), sm.group(2)
        cmds = []
        for cm in re.finditer(r"SceneCmd::(\w+)\s*\{([^}]*)\}", sm.group(3)):
            kind, args = cm.group(1), cm.group(2)
            kv = dict(
                (k.strip(), v.strip())
                for k, v in (p.split(":", 1) for p in args.split(",") if ":" in p)
            )
            if kind == "LightOnOff":
                cmds.append((0x40, [dev(kv["dev"]), 1 if kv["on"] == "true" else 0]))
            elif kind == "LightLevel":
                cmds.append((0x41, [dev(kv["dev"]), int(kv["level"])]))
            elif kind == "FanOnOff":
                cmds.append((0x42, [dev(kv["dev"]), 1 if kv["on"] == "true" else 0]))
            elif kind == "FanLevel":
                cmds.append((0x43, [dev(kv["dev"]), int(kv["level"])]))
            elif kind == "ThermoTarget":
                t = int(kv["temp_x10"])
                cmds.append((0x44, [dev(kv["zone"]), t & 0xFF, (t >> 8) & 0xFF]))
            elif kind == "ThermoMode":
                cmds.append((0x45, [dev(kv["zone"]), rust_consts[kv["mode"]]]))
            elif kind == "SocketOnOff":
                cmds.append((0x46, [dev(kv["dev"]), 1 if kv["on"] == "true" else 0]))
            else:
                raise ValueError(f"未知 SceneCmd 变体: {kind}")
        scenes[sid] = (name, cmds)
    return scenes


def parse_python_scenes(path: Path) -> dict:
    """解析 artpi_sim.py 的 SCENES 表：返回 {scene_id: (tag, name, [(cmd, payload)])}"""
    text = path.read_text(encoding="utf-8")
    cmd_defs = {}
    for m in re.finditer(r"^(CMD_\w+)\s*=\s*(0x[0-9A-Fa-f]+|\d+)",
                         text, re.M):
        cmd_defs[m.group(1)] = int(m.group(2), 0)
    block = re.search(r"^SCENES = \{(.*?)^\}", text, re.S | re.M).group(1)
    # 字符集预检后才 exec：只允许标识符/数字/字符串/容器/运算符，
    # 出现下划线开头名字（__import__ 等）直接拒。
    if not re.fullmatch(r"[\w\s()\[\],:'\".+\-]*", block):
        raise ValueError("SCENES 块含非预期字符，拒绝执行")
    if re.search(r"__\w+__", block):
        raise ValueError("SCENES 块含双下划线名字，拒绝执行")
    ns = {"struct": struct, "_tgt": lambda v: struct.pack("<h", v)}
    ns.update(cmd_defs)
    exec(f"SCENES = {{{block}}}", ns)  # noqa: S102 受信项目文件 + 字符集预检
    out = {}
    for sid, (tag, name, cmds) in ns["SCENES"].items():
        out[sid] = (tag, name, [(c, list(p)) for c, p in cmds])
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--artpi", type=Path, default=_first_existing(ART_PI_SCENE, ART_PI_SCENE_ALT))
    ap.add_argument("--rust", type=Path, default=RUST_SCENE)
    ap.add_argument("--sim", type=Path, default=SIM_SCENE)
    args = ap.parse_args()

    frame_h = _first_existing(ART_PI_FRAME_H, ART_PI_FRAME_H_ALT)
    cfg_h = _first_existing(ART_PI_SCENE.parent.parent / "inc" / "sh_config.h", ART_PI_CONFIG_H_ALT)
    types_h = _first_existing(ART_PI_SCENE.parent.parent / "inc" / "sh_types.h",
                              Path("D:/RT-ThreadStudio/workspace/MyART-PI/applications/inc/sh_types.h"))

    frame_defs = parse_defines(frame_h)
    cfg_defs = {**parse_defines(cfg_h), **parse_enum(types_h, "sh_thermo_mode_t")}
    rust_consts = {}
    for m in re.finditer(r"pub const (DEV_ID_\w+|THERMO_MODE_\w+):\s*u8\s*=\s*(\d+);",
                         RUST_CONSTANTS.read_text(encoding="utf-8")):
        rust_consts[m.group(1)] = int(m.group(2))

    c_scenes = parse_c_scenes(args.artpi.read_text(encoding="utf-8"), frame_defs, cfg_defs)
    rust_scenes = parse_rust_scenes(args.rust.read_text(encoding="utf-8"), rust_consts)
    sim_scenes = parse_python_scenes(args.sim)

    errors = []
    if set(c_scenes) != set(rust_scenes):
        errors.append(f"场景 id 集合不一致: C={sorted(c_scenes)} Rust={sorted(rust_scenes)}")
    if set(c_scenes) != set(sim_scenes):
        errors.append(f"场景 id 集合不一致: C={sorted(c_scenes)} Sim={sorted(sim_scenes)}")
    for sid in sorted(set(c_scenes) & set(rust_scenes)):
        tag, cname, c_cmds = c_scenes[sid]
        rname, r_cmds = rust_scenes[sid]
        if c_cmds != r_cmds:
            errors.append(f"场景 {sid}（C:{cname}/Rust:{rname}）命令序列不一致：\n"
                          f"  C   : {c_cmds}\n  Rust: {r_cmds}")
    for sid in sorted(set(c_scenes) & set(sim_scenes)):
        tag, cname, c_cmds = c_scenes[sid]
        _, sname, s_cmds = sim_scenes[sid]
        if c_cmds != s_cmds:
            errors.append(f"场景 {sid}（C:{cname}/Sim:{sname}）命令序列不一致：\n"
                          f"  C  : {c_cmds}\n  Sim: {s_cmds}")

    if errors:
        print("scene parity FAILED:")
        for e in errors:
            print(" -", e)
        return 1
    total = sum(len(v[2]) for v in c_scenes.values())
    print(f"scene parity OK: {len(c_scenes)} scenes / {total} commands 三端逐条一致"
          f"（ART-Pi sh_scene.c == Companion scene.rs == artpi_sim.py）")
    for sid in sorted(c_scenes):
        tag, name, cmds = c_scenes[sid]
        print(f"  {sid} {tag:<7} {name:<4} {len(cmds)} cmds")
    return 0


if __name__ == "__main__":
    sys.exit(main())
