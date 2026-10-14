import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../types";

interface Props {
  snapshot: Snapshot | null;
  onChanged: (s: Snapshot) => void;
}

/** 场景条：8 场景一键激活（与板载按键走同一条桥接代码路径）。 */
export function SceneBar({ snapshot, onChanged }: Props) {
  if (!snapshot) return null;
  const disabled = !snapshot.link.up;

  const activate = async (id: number) => {
    try {
      await invoke("activate_scene", { scene: id });
      const next = await invoke<Snapshot>("get_snapshot");
      onChanged(next);
    } catch {
      /* 错误已进事件日志 */
    }
  };

  return (
    <section className="card">
      <div className="card-header">
        <h2>场景</h2>
        <span className="hint">
          {disabled ? "ART-Pi 离线——命令会缓存重传" : "点击与按键等效（同一桥接路径）"}
        </span>
      </div>
      <div className="card-body">
        <div className="scene-grid">
          {snapshot.scenes.map((s) => (
            <button
              key={s.id}
              className={`scene-btn ${s.active ? "active" : ""}`}
              disabled={disabled}
              onClick={() => activate(s.id)}
              title={`${s.name}：${s.command_count} 条设备命令`}
            >
              <span className="name">{s.name}</span>
              <span className="meta">
                #{s.id} · {s.command_count} 条命令
              </span>
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}
