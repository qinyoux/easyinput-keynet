import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LogEntry, Snapshot } from "./types";
import { LinkStatus } from "./components/LinkStatus";
import { SceneBar } from "./components/SceneBar";
import { DeviceGrid } from "./components/DeviceGrid";
import { CommandInjector } from "./components/CommandInjector";
import { ConfigPanel } from "./components/ConfigPanel";
import { EventLog } from "./components/EventLog";

type Tab = "console" | "config";

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [tab, setTab] = useState<Tab>("console");

  useEffect(() => {
    // 启动：拉快照 + 补拉历史日志（install 之前的事件收不到）。
    invoke<Snapshot>("get_snapshot").then(setSnapshot).catch(() => {});
    invoke<LogEntry[]>("get_logs").then(setLogs).catch(() => {});

    // 快照广播（300ms 节流，见 lib.rs UDP 线程）。
    const unSnapshot = listen<Snapshot>("companion-snapshot", (e) => setSnapshot(e.payload));
    // 日志事件。
    const unLog = listen<LogEntry>("companion-log", (e) =>
      setLogs((prev) => [...prev.slice(-499), e.payload])
    );
    return () => {
      unSnapshot.then((f) => f());
      unLog.then((f) => f());
    };
  }, []);

  return (
    <div className="app">
      <header className="topbar">
        <div className="logo">
          EasyInput Companion<span>智能物联网中控 · 第8课大作业 MVP</span>
        </div>
        <div className="spacer" />
        <LinkStatus snapshot={snapshot} />
        <button
          className={`btn ghost`}
          onClick={() => {
            setTab("console");
            invoke("query_all").catch(() => {});
          }}
        >
          刷新状态
        </button>
        <button
          className={`btn ${tab === "config" ? "" : "ghost"}`}
          onClick={() => setTab(tab === "config" ? "console" : "config")}
        >
          {tab === "config" ? "返回控制台" : "配置"}
        </button>
      </header>

      {tab === "console" ? (
        <div className="main">
          <div className="col-left">
            <SceneBar snapshot={snapshot} onChanged={setSnapshot} />
            <DeviceGrid snapshot={snapshot} onChanged={setSnapshot} />
            <CommandInjector onDone={() => invoke<Snapshot>("get_snapshot").then(setSnapshot)} />
          </div>
          <div className="col-right">
            <EventLog logs={logs} />
          </div>
        </div>
      ) : (
        <div className="main">
          <div className="col-left">
            <ConfigPanel snapshot={snapshot} onChanged={setSnapshot} />
          </div>
          <div className="col-right">
            <EventLog logs={logs} />
          </div>
        </div>
      )}
    </div>
  );
}
