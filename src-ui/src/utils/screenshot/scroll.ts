/**
 * 滚动长截图前端辅助工具与格式化。
 */

export function formatPixelHeight(height: number): string {
  return `${height.toLocaleString()} px`;
}

export function formatConfidence(conf: number): string {
  return `${Math.round(conf * 100)}%`;
}
