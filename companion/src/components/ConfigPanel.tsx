import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "../types";

interface Props {
  snapshot: Snapshot | null;
  onChanged: (s: Snapshot) => void;
}

/**
 * 配置面板：ART-Pi 地址、场景模式、UUID 绑定表。
 *
 * UUID 列点击即复制——固件键位在 easyop 里配（0x10 配置通道），
 * 用户需要把这里的 UUID 粘到 easyop 的按键动作 `host_action:<uuid>` 上。
 */
export function ConfigPanel({ snapshot, onChanged }: Props) {
  const [ip, setIp] = useState(snapshot?.artpi_ip ?? "");
  // 兜底与后端默认一致（2026-10-16 起 remote）。前端写死"expand"会在
  // snapshot 未就绪时静默显示错模式，且诱导用户把展开模式存回配置。
  const [mode, setMode] = useState(snapshot?.scene_mode ?? "remote");
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    if (snapshot) {
      setIp(snapshot.artpi_ip);
      setMode(snapshot.scene_mode);
    }
  }, [snapshot]);

  const copy = async (uuid: string) => {
    try {
      await navigator.clipboard.writeText(uuid);
      setCopied(uuid);
      setTimeout(() => setCopied(null), 1200);
    } catch {
      /* 剪贴板不可用时静默（浏览器权限） */
    }
  };

  const applyIp = async () => {
    try {
      await invoke("set_artpi_ip", { ip });
      const next = await invoke<Snapshot>("get_snapshot");
      onChanged(next);
    } catch (e) {
      alert(String(e));
    }
  };

  const applyMode = async (next: string) => {
    setMode(next);
    try {
      // 读-改-存：get_map → 改字段 → update_map（Rust 侧 sanitize + 落盘 + 换目标）。
      const map = await invoke<Record<string, unknown>>("get_map");
      await invoke("update_map", { map: { ...map, scene_mode: next } });
      const snap = await invoke<Snapshot>("get_snapshot");
      onChanged(snap);
    } catch (e) {
      alert(String(e));
    }
  };

  if (!snapshot) return null;

  return (
    <section className="card">
      <div className="card-header">
        <h2>配置</h2>
        <span className="hint">保存到 %APPDATA%/com.easyinput.companion/home_map.json</span>
      </div>
      <div className="card-body config-panel">
        <div className="field">
          <label>ART-Pi 地址（UDP :17335，DHCP 地址以 FinSH ifconfig 为准）</label>
          <div className="raw-row">
            <input value={ip} onChange={(e) => setIp(e.target.value)} />
            <button className="btn" onClick={applyIp}>
              应用
            </button>
          </div>
        </div>

        <div className="field">
          <label>场景模式</label>
          <select value={mode} onChange={(e) => applyMode(e.target.value)}>
            <option value="expand">expand — Companion 展开成设备命令（MVP 默认）</option>
            <option value="remote">remote — 只发 0x47，由 ART-Pi 展开（M4 后启用）</option>
          </select>
          <div className="mode-note">
            expand 模式与 ART-Pi 当前的 <code>on_scene</code> 桩兼容：场景被展开成
            0x40–0x46 设备命令逐条发送，Ack 驱动重传；remote 模式留给 ART-Pi M4
            场景展开落地后切换，两条路不会双执行。
          </div>
        </div>

        <div className="field">
          <label>UUID 绑定表（点击复制 → 粘到 easyop 按键动作 host_action:&lt;uuid&gt;）</label>
          <table className="binding-table">
            <thead>
              <tr>
                <th>UUID</th>
                <th>动作</th>
              </tr>
            </thead>
            <tbody>
              {snapshot.bindings.map((b) => (
                <tr key={b.uuid}>
                  <td className="uuid" onClick={() => copy(b.uuid)} title="点击复制">
                    {copied === b.uuid ? "已复制 ✓" : b.uuid}
                  </td>
                  <td>{b.action}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
