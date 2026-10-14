import type { LogEntry } from "../types";

interface Props {
  logs: LogEntry[];
}

function fmtTs(ts: number): string {
  const d = new Date(ts);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

/** 事件日志：反向渲染（最新在上），warn/err 着色。 */
export function EventLog({ logs }: Props) {
  return (
    <section className="card" style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div className="card-header">
        <h2>事件日志</h2>
        <span className="hint">{logs.length} 条</span>
      </div>
      <div className="log-list">
        {logs.length === 0 && <div className="empty">暂无事件</div>}
        {[...logs].reverse().map((l, i) => (
          <div className={`log-line ${l.level}`} key={`${l.ts}-${i}`}>
            <span className="ts">{fmtTs(l.ts)}</span>
            <span className="msg">{l.message}</span>
          </div>
        ))}
      </div>
    </section>
  );
}
