/**
 * 滚动长截图前端辅助工具与格式化。
 */
import type { Rect } from './geometry';

export function formatPixelHeight(height: number): string {
  return `${height.toLocaleString()} px`;
}

export function formatConfidence(conf: number): string {
  return `${Math.round(conf * 100)}%`;
}

/**
 * 计算外侧边框绘制矩形，确保描边严格位于选区外围，绝不侵入选区内部污染真实图像。
 */
export function outerBorderRect(rect: Rect, lineWidth: number): Rect {
  const half = lineWidth / 2;
  return {
    x: rect.x - half,
    y: rect.y - half,
    width: rect.width + lineWidth,
    height: rect.height + lineWidth,
  };
}

/**
 * 判断当前是否处于长截图滚轮透传状态。
 */
export function shouldPassthroughWheel(liveMode: 'gif' | 'scroll' | null): boolean {
  return liveMode === 'scroll';
}
