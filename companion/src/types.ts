/** 与 Rust `state::Snapshot` 逐字段对应的前端类型。 */

export interface DeviceState {
  id: number;
  onoff: number;
  level: number;
  /** 0.1°C 定点 */
  temp_x10: number;
  err: number;
}

/**
 * 传感器无效哨兵值，与 ART-Pi `SH_THERMO_TEMP_X10_BAD(-1000)` 逐值对齐
 * （真源：MyART-PI/applications/inc/sh_config.h）。
 *
 * 板子 thermo_ctrl 在 `read_temp()` 连续失败时置 `err=1` + 该哨兵值，
 * 语义是「SHT30 没读到」，不是 -100℃ 实测值。验证期硬件未接传感器，
 * 恒命中此值——UI 必须显示「传感器未接」，把 -1000 当温度渲染成
 * 「-100.0℃」是把「没有数据」谎报成「有一个荒谬的数据」。
 */
export const TEMP_X10_BAD = -1000;

/** 温度展示文本：哨兵值 → 「传感器未接」，否则 x.x℃。 */
export function formatTemp(temp_x10: number): string {
  return temp_x10 === TEMP_X10_BAD ? "传感器未接" : `${(temp_x10 / 10).toFixed(1)}℃`;
}

export interface SceneView {
  id: number;
  name: string;
  command_count: number;
  active: boolean;
}

export interface BindingView {
  uuid: string;
  action: string;
}

export interface LinkInfo {
  up: boolean;
  peer: string;
  pending: number;
}

export interface Snapshot {
  link: LinkInfo;
  hid_online: boolean;
  active_scene: number | null;
  scenes: SceneView[];
  devices: DeviceState[];
  bindings: BindingView[];
  scene_mode: string;
  artpi_ip: string;
}

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "err";
  message: string;
}

/** 调试注入台的预设命令（cmd 数值与 ART-Pi sh_frame.h 一致）。 */
export const PRESET_COMMANDS: { label: string; cmd: number; payload: number[] }[] = [
  { label: "主灯 开", cmd: 0x40, payload: [1, 1] },
  { label: "主灯 关", cmd: 0x40, payload: [1, 0] },
  { label: "主灯 50%", cmd: 0x41, payload: [1, 50] },
  { label: "灯带 开", cmd: 0x40, payload: [2, 1] },
  { label: "灯带 80%", cmd: 0x41, payload: [2, 80] },
  { label: "风扇 开", cmd: 0x42, payload: [3, 1] },
  { label: "风扇 60%", cmd: 0x43, payload: [3, 60] },
  { label: "温控 24℃", cmd: 0x44, payload: [4, 0xf0, 0x00] },
  { label: "温控 自动", cmd: 0x45, payload: [4, 3] },
  { label: "温控 关", cmd: 0x45, payload: [4, 0] },
  { label: "插座1 开", cmd: 0x46, payload: [5, 1] },
  { label: "查询全部", cmd: 0x48, payload: [] },
];

export const DEVICE_NAMES: Record<number, string> = {
  1: "主灯",
  2: "灯带",
  3: "风扇",
  4: "温控",
  5: "插座1",
  6: "插座2",
};

export const THERMO_MODE_NAMES: Record<number, string> = {
  0: "关",
  1: "制冷",
  2: "制热",
  3: "自动",
};
