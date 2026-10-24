/**
 * 增强截图前端 API：小数据走统一 JSON IPC，冻结帧/贴图/标注层走 Tauri 二进制命令。
 */
import { invokeIpc, type IpcResponse } from './ipc';
import type { Rect } from '../utils/screenshot/geometry';

export type CapabilityState =
  | { state: 'available' }
  | { state: 'disabled' }
  | { state: 'unavailable'; reason: string };

export interface ScreenshotSessionInfo {
  session_id: string;
  frame_width: number;
  frame_height: number;
  frame_byte_length: number;
  device_pixel_ratio: number;
  logical_width: number;
  logical_height: number;
  monitor_id: number;
  monitor_name: string;
  window_region_count: number;
  capabilities: {
    window_tree: CapabilityState;
    ui_elements: CapabilityState;
    visual_edges: CapabilityState;
  };
  settings: {
    manual_selection: boolean;
    smart_regions: boolean;
    wheel_expand: boolean;
    magnifier: boolean;
    default_color: string;
    default_stroke_width: number;
  };
  created_at: number;
}

export interface DetectedRegion extends Rect {
  region_id: number;
  parent_id: number | null;
  level: 'ui_element' | 'container_panel' | 'top_level_window' | 'monitor';
  label: string | null;
}

export interface PinSummary {
  pin_id: string;
  label: string;
  pixel_width: number;
  pixel_height: number;
  logical_width: number;
  logical_height: number;
  opacity: number;
  png_byte_length: number;
  created_at: number;
}

export type ExportAction = 'copy' | 'save' | 'pin';

export interface ExportOutcome {
  action: ExportAction;
  width: number;
  height: number;
  saved_path: string | null;
  copied: boolean;
  clipboard_error: string | null;
  pin: PinSummary | null;
  at: number;
}

export interface ScreenshotFailure {
  kind: 'permission_denied' | 'unsupported' | 'not_found' | 'native_call_failed' | 'internal';
  capability: string | null;
  message: string;
  guidance: string | null;
  at: number;
}

export interface ScreenshotSettings {
  save_directory: string | null;
  file_name_prefix: string;
  copy_after_save: boolean;
  default_color: string;
  default_stroke_width: number;
  default_pin_opacity: number;
  max_pins: number;
  [extra: string]: unknown;
}

export interface ScreenshotStatus {
  session: { session_id: string; monitor_name: string; overlay_visible: boolean; created_at: number } | null;
  pins: PinSummary[];
  last_export: ExportOutcome | null;
  last_error: ScreenshotFailure | null;
  permission: { granted: boolean; message: string | null; guidance: string | null };
  shortcut: { action_id: string; registered: boolean; display_label: string | null; logical_hotkey: string | null };
  native_shortcut_error: string | null;
  save_directory: string;
  settings: ScreenshotSettings;
}

export interface OverlayFlags {
  enable_manual_box_selection: boolean;
  enable_smart_region_detection: boolean;
  smart_region_wheel_expand: boolean;
  show_magnifier: boolean;
}

export interface MonitorInfo {
  id: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale_factor: number;
  is_primary: boolean;
}

function tauriCore(): any {
  const core = (window as any).__TAURI__?.core;
  if (typeof core?.invoke !== 'function') {
    throw new Error('截图功能只能在桌面应用中使用');
  }
  return core;
}

/** Tauri 原始字节响应在 JS 侧为 ArrayBuffer；兼容旧运行时返回数字数组的情况。 */
function toArrayBuffer(value: unknown): ArrayBuffer {
  if (value instanceof ArrayBuffer) return value;
  if (ArrayBuffer.isView(value)) {
    const view = value as ArrayBufferView;
    return view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength) as ArrayBuffer;
  }
  if (Array.isArray(value)) return Uint8Array.from(value).buffer;
  throw new Error('二进制响应格式无效');
}

export async function fetchFrozenFrame(sessionId: string): Promise<ArrayBuffer> {
  return toArrayBuffer(await tauriCore().invoke('screenshot_frame', { sessionId }));
}

export async function fetchPinImage(pinId: string): Promise<ArrayBuffer> {
  return toArrayBuffer(await tauriCore().invoke('screenshot_pin_image', { pinId }));
}

/** 导出选区：标注层 PNG 作为原始请求体（无标注时为空），参数放在请求头。 */
export async function exportSelection(
  sessionId: string,
  selection: Rect,
  action: ExportAction,
  overlayPng: Uint8Array | null
): Promise<ExportOutcome> {
  const meta = JSON.stringify({ session_id: sessionId, selection, action });
  const res: IpcResponse<ExportOutcome> = await tauriCore().invoke(
    'screenshot_export',
    overlayPng ?? new Uint8Array(0),
    { headers: { 'x-aio-screenshot-export': meta } }
  );
  if (!res.ok || !res.data) throw new Error(res.error || '导出失败');
  return res.data;
}

export const screenshotApi = {
  getSession: (sessionId?: string) =>
    invokeIpc<ScreenshotSessionInfo>('screenshot_get_session', sessionId ? { session_id: sessionId } : {}),
  overlayReady: (sessionId: string) => invokeIpc('screenshot_overlay_ready', { session_id: sessionId }),
  resolveRegion: (sessionId: string, x: number, y: number, levelSteps: number) =>
    invokeIpc<DetectedRegion | null>('screenshot_resolve_region', {
      session_id: sessionId,
      x,
      y,
      level_steps: levelSteps,
    }),
  cancel: (sessionId?: string) =>
    invokeIpc<{ cancelled: boolean }>('screenshot_cancel', sessionId ? { session_id: sessionId } : {}),
  copyColor: (text: string) => invokeIpc('screenshot_copy_color', { text }),
  status: () => invokeIpc<ScreenshotStatus>('screenshot_get_status'),
  start: (monitorId?: number | null) =>
    invokeIpc<ScreenshotSessionInfo>('screenshot_start_capture', {
      hide_dashboard: true,
      ...(monitorId != null ? { monitor_id: monitorId } : {}),
    }),
  listMonitors: () => invokeIpc<MonitorInfo[]>('screenshot_list_monitors'),
  pinInfo: (pinId: string) => invokeIpc<PinSummary>('screenshot_pin_info', { pin_id: pinId }),
  closePin: (pinId: string) => invokeIpc('screenshot_close_pin', { pin_id: pinId }),
  closeAllPins: () => invokeIpc('screenshot_close_all_pins'),
  copyPin: (pinId: string) => invokeIpc('screenshot_pin_copy', { pin_id: pinId }),
  savePin: (pinId: string) => invokeIpc<{ saved_path: string }>('screenshot_pin_save', { pin_id: pinId }),
  getConfig: () => invokeIpc<{ settings: ScreenshotSettings; overlay: OverlayFlags }>('screenshot_get_config'),
  updateSettings: (settings: Partial<ScreenshotSettings>, overlay: Partial<OverlayFlags>) =>
    invokeIpc<{ settings: ScreenshotSettings; overlay: OverlayFlags }>('screenshot_update_settings', {
      settings,
      overlay,
    }),
};
