"""生成 Companion 的 home_map.json，与板子里的 smart-home 方案严格对齐。

**为什么改 Companion 侧而不是改板子侧**：板子方案已按「KEY1-8 = 8 场景」下发
（视频主演示的就是场景切换），而设备直控在 Companion UI 上本来就有 6 张卡片
可以直接点，优先级低于场景。两边必须一致，否则按了没反应。

对齐判据（与 `make_smart_home_profile.py` 严格对称）：
    KEY1..KEY8   ↔ scene 1..8      UUID ...0101..0108
    enc left/right/press ↔ encoder_left/right/encoder_light_toggle
                          UUID ...0301/0302/0303

另外把 artpi_ip 设为 127.0.0.1（本机跑 artpi-sim 联调），演示时改成
ART-Pi 实测 IP 即可 —— 改的是这一个字段，不用重下发板子配置。
"""

import json
import re
import sys
from pathlib import Path

COMPANION_CONSTANTS = Path(
    r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-keynet\companion\src-tauri\src\constants.rs"
)
HOME_MAP = Path(
    r"C:\Users\86136\AppData\Roaming\com.easyinput.companion\home_map.json"
)

SCENE_LABELS = ["回家", "离家", "睡眠", "影院", "阅读", "全开", "全关", "预冷"]

# Companion 工程根目录
COMPANION_ROOT = Path(r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-keynet\companion")
# `export-scenes` 是 Rust 真源导出器：`SceneCmd` 是带
# `#[serde(tag="op", rename_all="snake_case")]` 的枚举，手写必然对不上
# （实际踩过：我写了 `device_on` / `set_temp`，真值是 `light_on_off` /
# `thermo_target`）。对不上 ⇒ serde 整份 HomeMap 解析失败 ⇒ `load()`
# 静默回退内置默认 ⇒ artpi_ip 变回 192.168.1.10 ⇒ 帧发给空气，
# 日志里只有一行 warn。**scenes 永远从 Rust 导出，不手写。**
EXPORT_SCENES = COMPANION_ROOT / "src-tauri" / "target" / "debug" / "export-scenes.exe"


def load_scenes_from_rust() -> list:
    """调 `export-scenes` 拿内置场景真源；拿不到就明确失败，不猜。"""
    if not EXPORT_SCENES.exists():
        sys.exit(
            f"找不到 {EXPORT_SCENES}\n"
            f"先跑：cd {COMPANION_ROOT / 'src-tauri'} && cargo build --bin export-scenes"
        )
    import subprocess

    out = subprocess.run(
        [str(EXPORT_SCENES)], capture_output=True, text=True, encoding="utf-8"
    )
    if out.returncode != 0:
        sys.exit(f"export-scenes 失败：{out.stderr}")
    try:
        scenes = json.loads(out.stdout)
    except json.JSONDecodeError as e:
        sys.exit(f"export-scenes 输出不是合法 JSON：{e}\n前200 字：{out.stdout[:200]}")
    if len(scenes) != 8:
        sys.exit(f"内置场景数 {len(scenes)} != 8 —— Rust 侧变了？")
    return scenes


def extract() -> tuple[list[str], dict[str, str]]:
    src = COMPANION_CONSTANTS.read_text(encoding="utf-8")
    scenes = re.findall(
        r"^\s*(\d)\s*=>\s*Some\(\"(00000000-0000-4000-8000-0000000001\d\d)\"\),", src, re.M
    )
    enc = dict(
        re.findall(r'UUID_ENCODER_(LEFT|RIGHT|PRESS): &str = "([^"]+)"', src)
    )
    scene_uuids = [u for _, u in sorted(scenes, key=lambda x: int(x[0]))]
    if len(scene_uuids) != 8:
        sys.exit(f"场景 UUID 提取到 {len(scene_uuids)} 个（应为 8）")
    if len(enc) != 3:
        sys.exit(f"编码器 UUID 提取到 {len(enc)} 个（应为 3）")
    return scene_uuids, {k.lower(): v for k, v in enc.items()}


def main() -> None:
    scene_uuids, encoder = extract()

    bindings: list[dict] = []
    for i, uuid in enumerate(scene_uuids, 1):
        bindings.append({"uuid": uuid, "kind": "scene", "scene": i})
    bindings.append({"uuid": encoder["left"], "kind": "encoder_left"})
    bindings.append({"uuid": encoder["right"], "kind": "encoder_right"})
    bindings.append({"uuid": encoder["press"], "kind": "encoder_press"})

    # scenes 来自 Rust 真源（export-scenes），不手写——理由见 load_scenes_from_rust 注释
    default_scenes = load_scenes_from_rust()
    print("\n内置场景（Rust 真源导出）：")
    for s in default_scenes:
        ops = [c["op"] for c in s["commands"]]
        print(f"  {s['id']} {s['name']}: {', '.join(ops)}")

    cfg = {
        "artpi_ip": "127.0.0.1",
        "artpi_port": 17335,
        "scene_mode": "expand",
        "bindings": bindings,
        "scenes": default_scenes,
    }

    backup = HOME_MAP.with_suffix(".json.bak-smart-home")
    if HOME_MAP.exists() and not backup.exists():
        backup.write_text(HOME_MAP.read_text(encoding="utf-8"), encoding="utf-8")
        print(f"已备份 → {backup}")
    HOME_MAP.parent.mkdir(parents=True, exist_ok=True)
    HOME_MAP.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"\nhome_map.json 已写入：{len(bindings)} 条绑定")
    for i, b in enumerate(bindings, 1):
        tag = (
            f"场景{i}·{SCENE_LABELS[i-1]}"
            if b["kind"] == "scene"
            else {"encoder_left": "编码器左旋", "encoder_right": "编码器右旋",
                  "encoder_press": "编码器按下"}[b["kind"]]
        )
        print(f"  {tag:<14} {b['uuid']}")
    print(f"\nartpi_ip = {cfg['artpi_ip']}（本机模拟器；实机改成ART-Pi 的 IP）")
    print("scene_mode = expand（Companion 侧展开成设备命令，ART-Pi 的 0x47 是 TODO(M4) 桩）")


if __name__ == "__main__":
    main()