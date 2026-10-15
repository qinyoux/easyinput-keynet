"""路线 A 下发前的干跑验证：不碰开发板，纯逻辑复刻两侧判据。

两侧都要过才有意义：

1. **主机侧**（easyop `payload::action_to_firmware`）：`ActionSpec::HostAction`
   必须**原样透传**成 `host_action:<uuid>` 裸字符串。若它被改写（例如转成
   `{"hotkey":...}`），板子就不会发 kind 0x05 报文。
2. **固件侧**（`is_canonical_host_action_value` + `parse_action`）：
   前缀 `host_action:` + 36 字符 + 连字符在 8/13/18/23 + 其余小写 hex。
   任一条不过 → `UnknownAction` → **整份配置下发失败**（不是单项失败）。

再叠一层**跨程序一致性**：板子发出来的 UUID 必须与 Companion
`home_map.json` 里 bindings 用的 UUID 逐字相同。这一条错了，按键会
「按下完全没反应」，且没有任何错误日志——最难查的一类。
"""

import json
import re
import sys
from pathlib import Path

HOME_MAP = Path(
    r"C:\Users\86136\AppData\Roaming\com.easyinput.companion\home_map.json"
)
PROFILES = Path(
    r"C:\Users\86136\AppData\Roaming\com.easyop.app\profiles.json"
)
# Companion Rust 侧 `Action` 枚举（config_store.rs:33）的 serde tag 真值。
# **这个清单必须和 Rust 枚举逐字一致**：写错一个 kind，serde 反序列化整份
# home_map.json 失败，`load()` 静默回退内置默认 —— 8 个场景绑定全丢，
# 表现是「按了没反应」，日志里什么都没有。
COMPANION_ACTION_KINDS = {
    "scene",
    "device_toggle",
    "encoder_left",
    "encoder_right",
    "encoder_press",
}
COMPANION_SRC = Path(
    r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-keynet\companion\src-tauri\src\config_store.rs"
)

PASS, FAIL = "PASS", "FAIL"
results: list[tuple[str, str, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    results.append((PASS if ok else FAIL, name, detail))


def firmware_accepts(action_value: str) -> tuple[bool, str]:
    """逐行对照 C++ 源码判据。"""
    prefix = "host_action:"
    if not action_value.startswith(prefix):
        return False, "缺少 host_action: 前缀"
    uuid = action_value[len(prefix):]
    if len(uuid) != 36:
        return False, f"UUID 长度 {len(uuid)} != 36"
    for i, c in enumerate(uuid):
        if i in (8, 13, 18, 23):
            if c != "-":
                return False, f"第 {i} 位应为连字符，实为 '{c}'"
        elif not (c in "0123456789abcdef"):
            return False, f"第 {i} 位 '{c}' 非小写 hex"
    return True, "格式合法"


def main() -> None:
    profiles = json.loads(PROFILES.read_text(encoding="utf-8"))

    # ---- 0. 路径必须与 easyop 的 identifier 一致 ----
    # 踩过的坑：路径写成 `com.easyinput.keycfg`（9月2日老遗留目录），
    # 脚本"成功"、备份也做了，但 easyop 根本不读那个目录 ——
    # 界面上就是"看不到新方案"，且没有任何报错。
    conf = (
        Path(r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-prototype\src-tauri")
        / "tauri.conf.json"
    )
    m = re.search(r'"identifier"\s*:\s*"([^"]+)"', conf.read_text(encoding="utf-8"))
    expect_dir = m.group(1) if m else "?"
    check(
        f"路径与 easyop identifier({expect_dir})一致",
        PROFILES.parent.name == expect_dir,
        f"当前 {PROFILES.parent.name}",
    )
    check("方案库里有 7 个方案（原6 + 新增1）", len(profiles) == 7, f"实际 {len(profiles)}")

    smart = next((p for p in profiles if p.get("name") == "智能家居中控"), None)
    check("easyop profiles.json 含「智能家居中控」方案", smart is not None)
    if smart is None:
        report()
        return

    # ---- 1. 主机侧透传 ----
    bad_pass = []
    for key, spec in smart["keys"].items():
        v = spec["value"]
        if spec["kind"] != "host_action" or not v.startswith("host_action:"):
            bad_pass.append(f"{key}:{spec['kind']}")
    for k, spec in smart["encoder"].items():
        v = spec["value"]
        if spec["kind"] != "host_action" or not v.startswith("host_action:"):
            bad_pass.append(f"enc.{k}:{spec['kind']}")
    check("主机侧：11 个输入全为 host_action 裸字符串", not bad_pass, ",".join(bad_pass))

    # ---- 2. 固件侧判据 ----
    uuids = {}
    for key, spec in smart["keys"].items():
        ok, why = firmware_accepts(spec["value"])
        check(f"固件侧：{key} 接受", ok, why)
        uuids[key] = spec["value"][len("host_action:"):]
    for k, spec in smart["encoder"].items():
        ok, why = firmware_accepts(spec["value"])
        check(f"固件侧：encoder.{k} 接受", ok, why)
        uuids[f"enc.{k}"] = spec["value"][len("host_action:"):]

    # ---- 3. 载荷大小 vs 固件上限 ----
    payload = {
        "schema": "ai_keyboard.v1",
        "target_platform": "windows",
        "profiles": [{
            "keys": {k: {"press": v["value"]} for k, v in smart["keys"].items()},
            "encoder": {k: v["value"] for k, v in smart["encoder"].items()},
        }],
    }
    blob = json.dumps(payload, separators=(",", ":"))
    size = len(blob.encode("utf-8"))
    cap = 2048  # config_status.h: "config_max_bytes":2048
    check(f"载荷 {size}B ≤ 固件上限 {cap}B", size <= cap, f"超出 {size - cap}B")

    # ---- 4. 跨程序一致性：板子 UUID vs Companion bindings ----
    if not HOME_MAP.exists():
        check("Companion home_map.json 存在", False, f"{HOME_MAP} 不存在")
    else:
        hm = json.loads(HOME_MAP.read_text(encoding="utf-8"))
        bound = set()
        for b in hm.get("bindings", []):
            # bindings 里 uuid 可能被包在 kind 里，用递归找 36 段 hex
            def walk(node):
                if isinstance(node, str):
                    m = re.fullmatch(
                        r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
                        node,
                    )
                    if m:
                        bound.add(node)
                elif isinstance(node, dict):
                    for v in node.values():
                        walk(v)
                elif isinstance(node, list):
                    for v in node:
                        walk(v)
            walk(b)
        check(
            f"Companion 已绑定 {len(bound)} 个 UUID",
            len(bound) > 0,
            "home_map.json 里没找到 UUID",
        )
        overlap = bound & set(uuids.values())
        check(
            f"板子配置 ∩ Companion 映射 = {len(overlap)} 个",
            len(overlap) == 11,
            f"实际交集 {sorted(overlap)}",
        )

    # ---- 5. 与 easyop 保留段撞车检查 ----
    easyop_reserved = {
        "00000000-0000-4000-8000-000000000009",
        "00000000-0000-4000-8000-00000000000a",
        "00000000-0000-4000-8000-00000000000b",
        "00000000-0000-4000-8000-00000000000c",
        "00000000-0000-4000-8000-00000000000d",
        "00000000-0000-4000-8000-00000000000e",
        "00000000-0000-4000-8000-00000000000f",
        "00000000-0000-4000-8000-000000000010",
    }
    clash = set(uuids.values()) & easyop_reserved
    check("与 easyop 保留段(01–10)无撞车", not clash, str(sorted(clash)))

    # ---- 6. Companion 侧 kind 必须是 Rust 枚举里的真值 ----
    if HOME_MAP.exists():
        hm2 = json.loads(HOME_MAP.read_text(encoding="utf-8"))
        used = {b.get("kind") for b in hm2.get("bindings", [])}
        unknown = used - COMPANION_ACTION_KINDS
        check(
            f"Companion bindings 的 kind 全部合法（用到 {sorted(used)}）",
            not unknown,
            f"未知 kind {sorted(unknown)} —— Rust 枚举里没有，反序列化会整份失败",
        )
        # 反向核对：Rust 源码里真的还有这些变体吗（防枚举改了配置没跟）。
        # serde 的 `rename_all = "snake_case"` 把 `EncoderLeft` ↔ `encoder_left`，
        # 所以要把 tag 还原成 Rust 的 PascalCase 变体名再 grep。
        rust_src = COMPANION_SRC.read_text(encoding="utf-8")
        dead = []
        for k in used:
            if k in ("scene", "device_toggle"):
                continue  # 这两个是带字段的变体（Scene { scene }），单独查
            variant = "".join(w.capitalize() for w in k.split("_"))
            if not re.search(rf"\b{variant}\b", rust_src):
                dead.append(f"{k}→{variant}")
        check(
            "Rust 源码里存在这些变体（枚举未漂移）",
            not dead,
            f"Rust 里找不到：{dead}",
        )

        # 场景编号必须在 1..=8（SceneManager 按数组下标取）
        scenes_used = [b["scene"] for b in hm2.get("bindings", []) if b.get("kind") == "scene"]
        check(
            f"场景编号 {scenes_used} 全在 1..=8",
            all(1 <= s <= 8 for s in scenes_used),
            "越界会让 SceneManager 取不到场景",
        )
        check(
            f"8 个场景 UUID 与板子侧一致",
            set(scenes_used) == set(range(1, 9)),
            f"只绑了 {sorted(scenes_used)}",
        )

    report()


def report() -> None:
    width = max(len(n) for _, n, _ in results) + 2
    for status, name, detail in results:
        mark = "OK  " if status == PASS else "FAIL"
        line = f"[{mark}] {name.ljust(width)}"
        if detail and status == FAIL:
            line += f"← {detail}"
        print(line)
    npass = sum(1 for s, _, _ in results if s == PASS)
    nfail = len(results) - npass
    print(f"\n{npass} 通过 / {nfail} 失败 / 共 {len(results)}")
    sys.exit(0 if nfail == 0 else 1)


if __name__ == "__main__":
    main()