/**
 * 冻结帧取色（纯函数）：直接读取后端返回的 RGBA 原始像素，不经 Canvas 读回，结果无色彩管理偏差。
 */

export interface PixelColor {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** 读取帧像素颜色；坐标越界时返回 null。 */
export function readPixel(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  x: number,
  y: number
): PixelColor | null {
  if (!Number.isInteger(x) || !Number.isInteger(y) || x < 0 || y < 0 || x >= width || y >= height) {
    return null;
  }
  const i = (y * width + x) * 4;
  if (i + 3 >= rgba.length) return null;
  return { r: rgba[i], g: rgba[i + 1], b: rgba[i + 2], a: rgba[i + 3] };
}

/** `#RRGGBB`（大写，与后端校验格式一致）。 */
export function toHex(c: PixelColor): string {
  const h = (v: number) => v.toString(16).padStart(2, '0');
  return `#${h(c.r)}${h(c.g)}${h(c.b)}`.toUpperCase();
}

/** `rgb(r, g, b)`（与后端校验格式一致）。 */
export function toRgbText(c: PixelColor): string {
  return `rgb(${c.r}, ${c.g}, ${c.b})`;
}
