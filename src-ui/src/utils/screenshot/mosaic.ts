/**
 * 马赛克像素块计算（纯函数）：按块对冻结帧原始像素取平均色。
 *
 * 只读取原始帧，不读取其他标注，保证遮罩、预览与导出结果一致；
 * 每块颜色为块内全部像素均值，原文不会从块中残留。
 */
import type { Rect } from './geometry.ts';

export interface MosaicBlocks {
  cols: number;
  rows: number;
  /** cols × rows 的 RGBA，每个元素代表一个块。 */
  rgba: Uint8ClampedArray;
}

export function pixelateBlocks(
  frame: Uint8ClampedArray,
  frameWidth: number,
  frameHeight: number,
  rect: Rect,
  block: number
): MosaicBlocks {
  const size = Math.max(1, Math.floor(block));
  const x0 = Math.max(0, Math.floor(rect.x));
  const y0 = Math.max(0, Math.floor(rect.y));
  const x1 = Math.min(frameWidth, Math.ceil(rect.x + rect.width));
  const y1 = Math.min(frameHeight, Math.ceil(rect.y + rect.height));
  const cols = Math.max(0, Math.ceil((x1 - x0) / size));
  const rows = Math.max(0, Math.ceil((y1 - y0) / size));
  const out = new Uint8ClampedArray(cols * rows * 4);
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const bx0 = x0 + col * size;
      const by0 = y0 + row * size;
      const bx1 = Math.min(x1, bx0 + size);
      const by1 = Math.min(y1, by0 + size);
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      let n = 0;
      for (let y = by0; y < by1; y++) {
        let i = (y * frameWidth + bx0) * 4;
        for (let x = bx0; x < bx1; x++, i += 4) {
          r += frame[i];
          g += frame[i + 1];
          b += frame[i + 2];
          a += frame[i + 3];
          n++;
        }
      }
      const o = (row * cols + col) * 4;
      if (n > 0) {
        out[o] = Math.round(r / n);
        out[o + 1] = Math.round(g / n);
        out[o + 2] = Math.round(b / n);
        out[o + 3] = Math.round(a / n);
      }
    }
  }
  return { cols, rows, rgba: out };
}
