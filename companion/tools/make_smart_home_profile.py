"""为 Companion 生成 easyop 方案（路线 A：借 easyop 的 0x10 下发通道刷板子 keymap）。

设计要点（踩过的坑都在这里）：

1. **UUID 不手抄**，从 `constants.rs` 正则提取。字面不一致 = 板子发的 UUID 和
   Companion `home_map.json` 对不上 = 按键「按下没反应」，且没有任何报错。
2. **追加而不是覆盖** `profiles.json`：钦友原来的「默认方案」（语音/复制/粘贴）
   必须保留，否则回不去。
3. **每项都用 `host_action:<uuid>` 裸字符串**：`payload::to_firmware_value` 对
   `ActionSpec::HostAction` 是原样透传，固件侧 `parse_action` 只做格式校验
   （前缀 + 36 位小写 hex + 连字符位），**没有 UUID 白名单** —— 这是路线 A
   能成立的根本原因。
4. 键位分工：KEY1-8 → 8 个场景，编码器 left/right/press → 亮度-10 / 亮度+10 /
   主灯切换。11 个输入全用上，不留废键。
"""

import json
import os
import re
import sys
from pathlib import Path

COMPANION_CONSTANTS = Path(
    r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-keynet\companion\src-tauri\src\constants.rs"
)
# ⚠️ 路径必须由 tauri.conf.json 的 `identifier` 决定，**不能靠猜**。
# easyop 的 identifier 是 `com.easyop.app` ⇒ `%APPDATA%\com.easyop.app`。
# 踩过的坑：我原先写成 `com.easyinput.keycfg`（一个 9月2日的老遗留目录），
# 脚本"成功"了、备份也做了，但 easyop 根本不读那个目录 —— 界面上当然看不到新方案。
EASYOP_PROFILES = Path(
    r"C:\Users\86136\AppData\Roaming\com.easyop.app\profiles.json"
)

# 8 个场景的语义标签（与 Companion `home/scene.rs::default_scenes()` 的顺序一致）
SCENE_LABELS = ["回家", "离家", "睡眠", "影院", "阅读", "全开", "全关", "预冷"]
ENCODER_LABELS = {"left": "主灯亮度 -10", "right": "主灯亮度 +10", "press": "主灯开关"}

# id 用标准 UUID 形态：现有 5 个方案全是 `uuid4()` 生成的（`98735a06-…`），
# 且 UI 的 `p.id === activeId` 比对、「新建方案」都按 UUID 语义处理。
# 这里用固定 UUID 而不是 `smart-home` 之类的可读串 —— 避免任何一处
# 假设 id 是 UUID 而踩坑。
PROFILE_ID = "5c3a1e77-9d42-4b18-a6f5-0e2c8b7d4a10"
PROFILE_NAME = "智能家居中控"


def extract_uuids() -> tuple[list[str], dict[str, str]]:
    """从 constants.rs 提取 UUID 字面量（唯一真源，避免手抄漂移）。"""
    src = COMPANION_CONSTANTS.read_text(encoding="utf-8")

    # 场景：形如 `1 => Some("00000000-...-0101")`
    scenes = re.findall(
        r"^\s*(\d)\s*=>\s*Some\(\"(00000000-0000-4000-8000-0000000001\d\d)\"\),",
        src,
        re.M,
    )
    # 设备：形如 `1 => Some("00000000-...-0201")`
    devices = re.findall(
        r"^\s*(\d)\s*=>\s*Some\(\"(00000000-0000-4000-8000-0000000002\d\d)\"\),",
        src,
        re.M,
    )
    # 编码器：`pub const UUID_ENCODER_LEFT: &str = "..."`
    enc = dict(
        re.findall(r'UUID_ENCODER_(LEFT|RIGHT|PRESS): &str = "([^"]+)"', src)
    )

    scene_uuids = [u for _, u in sorted(scenes, key=lambda x: int(x[0]))]
    device_uuids = [u for _, u in sorted(devices, key=lambda x: int(x[0]))]

    if len(scene_uuids) != 8:
        sys.exit(f"场景 UUID 提取到 {len(scene_uuids)} 个（应为 8）——constants.rs 结构变了？")
    if len(device_uuids) != 6:
        sys.exit(f"设备 UUID 提取到 {len(device_uuids)} 个（应为 6）")
    if len(enc) != 3:
        sys.exit(f"编码器 UUID 提取到 {len(enc)} 个（应为 3）")

    encoder = {k.lower(): v for k, v in enc.items()}
    print("从 constants.rs 提取：")
    for i, u in enumerate(scene_uuids, 1):
        print(f"  场景{i} {SCENE_LABELS[i-1]:<4} {u}")
    for i, u in enumerate(device_uuids, 1):
        print(f"  设备{i}            {u}")
    for k in ("left", "right", "press"):
        print(f"  编码器{k:<6} {ENCODER_LABELS[k]:<14} {encoder[k]}")

    return scene_uuids, device_uuids, encoder


def verify_canonical(uuid: str) -> None:
    """照抄固件 `is_canonical_host_action_value` 的判据预检。

    固件源码（C++）：
        前缀 `host_action:` + 36 字符；连字符在第 8/13/18/23 位；其余小写 hex。
    任一条不满足 → `parse_action` 返回 `UnknownAction` → 整份配置下发失败。
    """
    u = uuid
    assert len(u) == 36, f"UUID 长度 {len(u)} != 36：{u}"
    for i, c in enumerate(u):
        if i in (8, 13, 18, 23):
            assert c == "-", f"第 {i} 位应为连字符：{u}"
        else:
            assert c in "0123456789abcdef", f"第 {i} 位 '{c}' 非小写 hex：{u}"


def build_profile() -> dict:
    scene_uuids, device_uuids, encoder = extract_uuids()

    keys = {}
    for i, uuid in enumerate(scene_uuids, 1):
        verify_canonical(uuid)
        keys[f"KEY{i}"] = {
            "kind": "host_action",
            "value": f"host_action:{uuid}",
            "_label": f"{SCENE_LABELS[i-1]}场景",
        }
    for uuid in device_uuids:
        verify_canonical(uuid)

    encoder_spec = {}
    for k, uuid in encoder.items():
        verify_canonical(uuid)
        encoder_spec[k] = {
            "kind": "host_action",
            "value": f"host_action:{uuid}",
            "_label": ENCODER_LABELS[k],
        }

    return {
        "id": PROFILE_ID,
        "name": PROFILE_NAME,
        "keys": keys,
        "encoder": encoder_spec,
        "prompts": [],
    }


def strip_labels(node):
    """`_label` 是我加的注释字段，必须剥掉 —— easyop 的 ActionSpec 没有它，
    serde 遇到未知字段不会报错（默认忽略），但留着会让文件语义不干净。"""
    if isinstance(node, dict):
        return {k: strip_labels(v) for k, v in node.items() if not k.startswith("_")}
    return node


def resolve_easyop_profiles() -> Path:
    """从 easyop 的 tauri.conf.json 反推配置路径，绝不硬编码。

    `app_config_dir()`（easyop `commands.rs:59`）由 tauri 的 `identifier`
    决定 ⇒ `%APPDATA%/<identifier>`。identifier 改了路径就变，而写错路径的
    后果是**脚本"成功"了、easyop 却根本读不到**——不报错，只是界面里没有。

    这里做一次交叉验证：算出的目录必须存在，且里面必须有 `profiles.json`；
    否则直接退出，不做任何写入。
    """
    conf = (
        Path(r"E:\MY_AI\trae\ESP32_S3_BOT\easyinput-prototype\src-tauri")
        / "tauri.conf.json"
    )
    if not conf.exists():
        sys.exit(f"读不到 {conf}，无法确定 easyop 配置目录")
    m = re.search(r'"identifier"\s*:\s*"([^"]+)"', conf.read_text(encoding="utf-8"))
    if not m:
        sys.exit("tauri.conf.json 里没有 identifier 字段")
    identifier = m.group(1)

    appdata = Path(os.environ.get("APPDATA", "")) / identifier
    if not appdata.is_dir():
        sys.exit(
            f"推导出的配置目录不存在：{appdata}\n"
            f"（identifier={identifier}）——easyop 可能从未启动过？"
        )
    profiles = appdata / "profiles.json"
    if not profiles.exists():
        sys.exit(
            f"{appdata} 下没有 profiles.json。\n"
            f"请先启动一次 easyop 让它生成默认配置，再重跑本脚本。"
        )
    print(f"easyop identifier = {identifier}")
    print(f"profiles.json      = {profiles}")
    return profiles


def main() -> None:
    global EASYOP_PROFILES
    EASYOP_PROFILES = resolve_easyop_profiles()
    profile = strip_labels(build_profile())

    existing = json.loads(EASYOP_PROFILES.read_text(encoding="utf-8"))
    if isinstance(existing, dict):
        existing = existing.get("profiles", [])
    if not isinstance(existing, list):
        sys.exit("profiles.json 顶层既不是 list 也不是 {profiles:[...]}，需要人工确认")

    before = len(existing)
    # 同 id 覆盖，避免重复运行产生两个方案
    existing = [p for p in existing if p.get("id") != PROFILE_ID]
    existing.append(profile)

    backup = EASYOP_PROFILES.with_suffix(".json.bak-smart-home")
    if not backup.exists():
        backup.write_text(
            EASYOP_PROFILES.read_text(encoding="utf-8"), encoding="utf-8"
        )
        print(f"\n已备份原配置 → {backup}")

    EASYOP_PROFILES.write_text(
        json.dumps(existing, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    print(f"\nprofiles.json: {before} → {len(existing)} 个方案")
    print(f"新方案 id={PROFILE_ID} name={PROFILE_NAME}")
    print("  KEY1-8 → 8 个场景；编码器 left/right/press → 亮度-10/亮度+10/主灯开关")
    print("\n下一步：在 easyop 里选中「智能家居中控」方案 → 点『下发到开发板』")


if __name__ == "__main__":
    main()