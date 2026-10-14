import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../types";
import { DEVICE_NAMES } from "../types";

interface Props {
  snapshot: Snapshot | null;
  onChanged: (s: Snapshot) => void;
}

/** 设备网格：6 台设备的状态可视化 + 单机开关（权威状态来自 ART-Pi 回传）。 */
export function DeviceGrid({ snapshot, onChanged }: Props) {
  if (!snapshot) return null;

  const toggle = async (id: number) => {
    try {
      await invoke("toggle_device", { device: id });
      const next = await invoke<Snapshot>("get_snapshot");
      onChanged(next);
    } catch {
      /* 错误已进事件日志 */
    }
  };

  return (
    <section className="card">
      <div className="card-header">
        <h2>设备状态</h2>
        <span className="hint">状态来自 ART-Pi 回传帧（7B/设备）</span>
      </div>
      <div className="card-body">
        <div className="device-grid">
          {snapshot.devices.map((d) => {
            const on = d.onoff === 1;
            return (
              <div className="device-card" key={d.id}>
                <div className="row1">
                  <span className="dev-name">{DEVICE_NAMES[d.id] ?? `设备${d.id}`}</span>
                  <button
                    className={`switch ${on ? "on" : ""}`}
                    onClick={() => toggle(d.id)}
                    title={on ? "点击关闭" : "点击开启"}
                    aria-label={`${DEVICE_NAMES[d.id] ?? d.id} 开关`}
                  />
                </div>
                <div className="row2">
                  <span>{d.id === 4 ? "目标" : on ? "开启" : "关闭"}</span>
                  {d.id === 4 ? (
                    <span className="temp">{(d.temp_x10 / 10).toFixed(1)}℃</span>
                  ) : (
                    <>
                      <div className="bar">
                        <div style={{ width: `${d.level}%` }} />
                      </div>
                      <span>{d.level}%</span>
                    </>
                  )}
                </div>
                {d.err !== 0 && <span className="err-tag">故障码 {d.err}</span>}
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
