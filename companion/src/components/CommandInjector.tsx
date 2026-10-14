import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PRESET_COMMANDS } from "../types";

interface Props {
  onDone: () => void;
}

/**
 * 命令注入调试台：预设芯片 + 裸命令（cmd 十六进制 + payload 十进制数组）。
 *
 * 用途有两个：① 没有实体板时验证 PC↔ART-Pi 链路；② 演示视频里展示
 * "可视化 + 备用控制源" 这一 PC Companion 的本职角色。
 */
export function CommandInjector({ onDone }: Props) {
  const [cmdHex, setCmdHex] = useState("40");
  const [payload, setPayload] = useState("1,1");
  const [busy, setBusy] = useState(false);

  const sendRaw = async () => {
    setBusy(true);
    try {
      const cmd = Number.parseInt(cmdHex, 16);
      if (Number.isNaN(cmd) || cmd < 0 || cmd > 255) throw new Error("cmd 必须是 00-FF");
      const arr = payload
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
        .map((s) => {
          const n = Number.parseInt(s, 10);
          if (Number.isNaN(n) || n < 0 || n > 255) throw new Error(`payload 字节越界: ${s}`);
          return n;
        });
      await invoke("send_raw_command", { cmd, payload: arr });
      onDone();
    } catch (e) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  const sendPreset = async (cmd: number, payload: number[]) => {
    setBusy(true);
    try {
      await invoke("send_raw_command", { cmd, payload });
      onDone();
    } catch {
      /* 错误已进事件日志 */
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card">
      <div className="card-header">
        <h2>命令注入（调试台）</h2>
        <span className="hint">绕过按键直接发 V2.3 帧</span>
      </div>
      <div className="card-body injector">
        <div className="preset-row">
          {PRESET_COMMANDS.map((p) => (
            <button
              key={p.label}
              className="chip"
              disabled={busy}
              onClick={() => sendPreset(p.cmd, p.payload)}
              title={`cmd=0x${p.cmd.toString(16).padStart(2, "0")} payload=[${p.payload}]`}
            >
              {p.label}
            </button>
          ))}
        </div>
        <div className="raw-row">
          <span style={{ color: "var(--text-3)", fontSize: 12 }}>cmd 0x</span>
          <input
            value={cmdHex}
            onChange={(e) => setCmdHex(e.target.value)}
            style={{ maxWidth: 64 }}
            aria-label="命令码（十六进制）"
          />
          <span style={{ color: "var(--text-3)", fontSize: 12 }}>payload</span>
          <input
            value={payload}
            onChange={(e) => setPayload(e.target.value)}
            placeholder="1,1"
            aria-label="载荷（十进制字节，逗号分隔）"
          />
          <button className="btn" onClick={sendRaw} disabled={busy}>
            发送
          </button>
        </div>
      </div>
    </section>
  );
}
