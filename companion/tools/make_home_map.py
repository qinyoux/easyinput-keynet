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

    # 场景机内容沿用 Companion 内置的 8 场景默认（home/scene.rs::default_scenes），
    # 这里只做透传，让配置文件自解释——改场景行为请改 Rust 侧，那是唯一真源。
    default_scenes = [
        {"scene": 1, "label": "回家", "ops": [{"op": "device_on", "device": 1, "level": 80},
                                              {"op": "set_temp", "celsius": 24}]},
        {"scene": 2, "label": "离家", "ops": [{"op": "all_off"}]},
        {"scene": 3, "label": "睡眠", "ops": [{"op": "device_on", "device": 1, "level": 20},
                                              {"op": "device_off", "device": 3},
                                              {"op": "set_temp", "celsius": 26}]},
        {"scene": 4, "label": "影院", "ops": [{"op": "device_on", "device": 1, "level": 30},
                                              {"op": "device_on", "device": 3, "level": 40}]},
        {"scene": 5, "label": "阅读", "ops": [{"op": "device_on", "device": 1, "level": 90},
                                              {"op": "set_temp", "celsius": 23}]},
        {"scene": 6, "label": "全开", "ops": [{"op": "all_on"}]},
        {"scene": 7, "label": "全关", "ops": [{"op": "all_off"}]},
        {"scene": 8, "label": "预冷", "ops": [{"op": "set_temp", "celsius": 20},
                                              {"op": "device_on", "device": 3, "level": 60}]},
    ]

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