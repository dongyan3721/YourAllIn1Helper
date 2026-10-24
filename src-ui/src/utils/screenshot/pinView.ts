/**
 * 贴图窗口视图状态（纯函数）：缩放 10%–500%、透明度 10%–100%、双击折叠。
 */

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 5;
export const MIN_OPACITY = 0.1;
export const MAX_OPACITY = 1;
/** 折叠后的迷你色块边长（逻辑像素）。 */
export const COLLAPSED_SIZE = 40;
/** 窗口最小边长（逻辑像素），与宿主窗口预设一致。 */
export const MIN_WINDOW_SIZE = 16;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/** 滚轮一格缩放 10%（乘法步进），结果吸附到 1% 并限制在 10%–500%。 */
export function stepZoom(current: number, wheelDelta: number): number {
  if (wheelDelta === 0) return current;
  const factor = wheelDelta < 0 ? 1.1 : 1 / 1.1;
  return clamp(Math.round(current * factor * 100) / 100, MIN_ZOOM, MAX_ZOOM);
}

/** Shift + 滚轮一格调节 10% 透明度。 */
export function stepOpacity(current: number, wheelDelta: number): number {
  if (wheelDelta === 0) return current;
  const next = current + (wheelDelta < 0 ? 0.1 : -0.1);
  return clamp(Math.round(next * 10) / 10, MIN_OPACITY, MAX_OPACITY);
}

/** 取滚轮有效增量：macOS 按住 Shift 时 WebKit 会把纵向滚动转成横向，需要兼容 deltaX。 */
export function wheelDelta(event: { deltaX: number; deltaY: number }): number {
  return event.deltaY !== 0 ? event.deltaY : event.deltaX;
}

/** 当前应设置的窗口逻辑尺寸。 */
export function pinWindowSize(
  baseWidth: number,
  baseHeight: number,
  zoom: number,
  collapsed: boolean
): { width: number; height: number } {
  if (collapsed) return { width: COLLAPSED_SIZE, height: COLLAPSED_SIZE };
  const z = clamp(zoom, MIN_ZOOM, MAX_ZOOM);
  return {
    width: Math.max(MIN_WINDOW_SIZE, Math.round(baseWidth * z)),
    height: Math.max(MIN_WINDOW_SIZE, Math.round(baseHeight * z)),
  };
}

export function formatPercent(value: number): string {
  return `${Math.round(value * 100)}%`;
}
