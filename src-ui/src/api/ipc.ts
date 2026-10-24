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

export type MediaJobStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface MediaJob {
  id: string;
  operation: string;
  input_path: string;
  output_path: string | null;
  status: MediaJobStatus;
  progress: number;
  speed: string | null;
  error: string | null;
  metadata: unknown;
  created_at: number;
  updated_at: number;
}

export interface MediaToolProbe {
  tool: string;
  available: boolean;
  resolved_path: string | null;
  version_summary: string | null;
  from_custom_path: boolean;
  install_hint: string;
}

export interface MediaEnvironment {
  ffmpeg: MediaToolProbe;
  ffprobe: MediaToolProbe;
  image_formats: Array<'png' | 'jpeg' | 'webp' | 'bmp' | 'ico'>;
  av_operations: string[];
}

export interface MediaProcConfig {
  output_dir: string | null;
  max_concurrent_jobs: number;
  history_limit: number;
}

export interface MediaJobProgress {
  job_id: string;
  progress: number;
  speed: string | null;
  status: MediaJobStatus;
  job: MediaJob;
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


/** 脚本类型（与 Rust `ScriptType` 的 snake_case 序列化一致） */
export type ScriptKind =
  | 'python'
  | 'node'
  | 'shell'
  | 'applescript'
  | 'powershell'
  | 'batch'
  | 'vbscript';

/** 解释器探测结果 */
export interface ScriptInterpreter {
  script_type: ScriptKind;
  supported: boolean;
  available: boolean;
  resolved_path: string | null;
  version_summary: string | null;
  install_hint: string;
  unavailable_reason: string | null;
}

/** AIO_CLIPBOARD / AIO_SELECTION 实际注入能力 */
export interface ScriptEnvCapabilities {
  clipboard: boolean;
  selection: boolean;
  detail: string;
}

/** 脚本库记录 */
export interface ScriptRecord {
  id: number;
  name: string;
  script_type: ScriptKind;
  is_inline: boolean;
  content_or_path: string;
  args: string[];
  working_dir: string | null;
  hotkey: string | null;
  show_in_tray: boolean;
  timeout_secs: number;
  output_handling: 'stream' | 'final' | 'discard' | string;
  created_at: number;
  updated_at: number;
  hotkey_active: boolean;
}

export type ScriptRunStatus =
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'timed_out'
  | 'cancelled'
  | 'spawn_failed'
  | 'interrupted';

/** 脚本执行记录 */
export interface ScriptRunRecord {
  run_id: string;
  script_id: number;
  script_name: string;
  status: ScriptRunStatus;
  exit_code: number | null;
  timed_out: boolean;
  pid: number | null;
  started_at: number;
  finished_at: number | null;
  duration_ms: number | null;
  stdout_tail: string;
  stderr_tail: string;
  error_message: string | null;
  log_bytes: number;
  dropped_bytes: number;
}

/** 增量日志条目 */
export interface ScriptLogEntry {
  seq: number;
  stream: 'stdout' | 'stderr';
  text: string;
  at: number;
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
