import type { Snapshot } from "../types";

interface Props {
  snapshot: Snapshot | null;
}

/** 顶栏链路状态：HID（板↔PC）与 UDP（PC↔ART-Pi）两条链路各自一盏灯。 */
export function LinkStatus({ snapshot }: Props) {
  if (!snapshot) {
    return (
      <div className="link-pills">
        <span className="pill">
          <span className="dot" />
          初始化…
        </span>
      </div>
    );
  }
  const { hid_online, link } = snapshot;
  return (
    <div className="link-pills">
      <span className={`pill ${hid_online ? "ok" : "err"}`} title="EasyInput 开发板 USB HID">
        <span className="dot" />
        HID {hid_online ? "在线" : "离线"}
      </span>
      <span
        className={`pill ${link.up ? "ok" : "err"}`}
        title={`ART-Pi ${link.peer}${link.pending > 0 ? `，${link.pending} 条命令待确认` : ""}`}
      >
        <span className="dot" />
        ART-Pi {link.up ? "在线" : "离线"}
      </span>
      {link.pending > 0 && (
        <span className="pill warn" title="等待 Ack 的命令数（超过 3s 未确认会自动重传/失败）">
          <span className="dot" />
          待确认 {link.pending}
        </span>
      )}
    </div>
  );
}
