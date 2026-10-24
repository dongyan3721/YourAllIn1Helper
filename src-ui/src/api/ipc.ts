/**
 * Tauri 2.x 微内核强类型 IPC 通信适配层
 * 统一处理前端向 Rust 发送的 Command，并封装统一的 IpcResponse 回包。
 */

/** IPC 响应信封结构 */
export interface IpcResponse<T = any> {
  ok: boolean;
  data?: T;
  error?: string;
}

/** 剪贴板条目数据结构 */
export interface ClipboardRecord {
  id: number;
  item_type: 'text' | 'image';
  content_text: string | null;
  blob_rel_path: string | null;
  thumb_base64: string | null;
  content_hash: string;
  source_app_name: string | null;
  is_pinned: boolean;
  created_at: number;
  updated_at: number;
}

/** 桌面便笺数据结构 */
export interface StickyNoteRecord {
  id: string;
  title: string;
  content_md: string;
  theme_color: string;
  opacity: number;
  pos_x: number | null;
  pos_y: number | null;
  width: number;
  height: number;
  is_pinned: boolean;
  is_open: boolean;
  updated_at: number;
}

/** 外部窗口静态快照信息 */
export interface ExternalWindowInfo {
  handle: number;
  title: string;
  process_name: string;
  process_id: number;
  x: number;
  y: number;
  width: number;
  height: number;
  is_topmost: boolean;
}

/** 已置顶外部窗口条目 */
export interface PinnedWindowEntry {
  info: ExternalWindowInfo;
  pinned_at: number;
}

export interface ModuleStatus {
  number: number;
  id: string;
  name: string;
  available: boolean;
  enabled: boolean;
  supported: boolean;
  unavailable_reason: string;
}

/** 微内核运行状态概览 */
export interface KernelStatus {
  target_os: string;
  module_catalog: ModuleStatus[];
  active_window_count: number;
  active_windows: Array<{
    spec: {
      label: string;
      route: string;
      title: string;
      policy?: {
        transient?: { destroy_on_blur: boolean };
      };
    };
    always_on_top: boolean;
  }>;
  modules: Array<{
    id: string;
    name: string;
  }>;
  shortcuts: Array<{
    spec: {
      module_id: string;
      action_id: string;
    };
    active_logical_hotkey: string;
    physical_accelerator: string;
    display_label: string;
  }>;
  config: {
    general?: {
      launch_at_login: boolean;
      show_tray_icon: boolean;
      language: string;
      theme: string;
    };
    clipboard: {
      enabled: boolean;
      max_history_items: number;
      capture_images: boolean;
    };
  };
  clipboard_suppressed: boolean;
  paste_trigger_count: number;
  foreground_external_window: ExternalWindowInfo | null;
  env_tools: Array<{
    tool: string;
    available: boolean;
    resolved_path: string | null;
    version_summary: string | null;
  }>;
}

/**
 * 调用微内核统一 IPC 分发器
 * @param cmd 命令名称
 * @param args 参数对象
 * @returns 成功返回 data，失败抛出 Error
 */
export async function invokeIpc<T = any>(cmd: string, args: Record<string, any> = {}): Promise<T> {
  let resPayload: IpcResponse<T>;

  // 检测是否运行在 Tauri 2.x 原生 WebView 容器中
  const tauriInvoke = (window as any).__TAURI__?.core?.invoke;
  if (typeof tauriInvoke === 'function') {
    resPayload = await tauriInvoke('ipc', {
      request: { cmd, args },
    });
  } else {
    // 降级回退到本地 HTTP 开发桥接服务
    const response = await fetch('/api/ipc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ cmd, args }),
    });
    resPayload = await response.json();
  }

  if (!resPayload.ok) {
    throw new Error(resPayload.error || `IPC 命令 '${cmd}' 执行失败`);
  }

  return resPayload.data as T;
}
