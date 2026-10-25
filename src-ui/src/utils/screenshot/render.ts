/**
 * 标注的 Canvas 2D 渲染。遮罩预览与导出标注层共用同一套绘制函数，保证所见即所得。
 */
import type { Annotation, MosaicAnnotation } from './annotations.ts';
import { numberRadius, textFontSize } from './annotations.ts';
import { pixelateBlocks } from './mosaic.ts';

/** 冻结帧原始像素，用于马赛克取色。 */
export interface FrameSource {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
}

/** 马赛克块缓存：标注创建后内容不变，按 ID 缓存放大前的块画布。 */
export type MosaicCache = Map<string, HTMLCanvasElement>;

export const TEXT_FONT_FAMILY =
  '-apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans", "Microsoft YaHei", "Segoe UI", sans-serif';
export const TEXT_LINE_HEIGHT = 1.25;

export function renderAnnotations(
  ctx: CanvasRenderingContext2D,
  items: readonly Annotation[],
  frame: FrameSource,
  cache: MosaicCache
): void {
  for (const item of items) renderAnnotation(ctx, item, frame, cache);
}

export function renderAnnotation(
  ctx: CanvasRenderingContext2D,
  a: Annotation,
  frame: FrameSource,
  cache: MosaicCache
): void {
  ctx.save();
  ctx.strokeStyle = a.color;
  ctx.fillStyle = a.color;
  ctx.lineWidth = a.size;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  switch (a.kind) {
    case 'rect':
      ctx.strokeRect(a.rect.x + a.size / 2, a.rect.y + a.size / 2, Math.max(0, a.rect.width - a.size), Math.max(0, a.rect.height - a.size));
      break;
    case 'arrow':
      drawArrow(ctx, a.from.x, a.from.y, a.to.x, a.to.y, a.size);
      break;
    case 'mosaic':
      drawMosaic(ctx, a, frame, cache);
      break;
    case 'number':
      drawNumber(ctx, a.at.x, a.at.y, a.value, a.size, a.color);
      break;
    case 'text':
      drawText(ctx, a.at.x, a.at.y, a.text, a.size);
      break;
  }
  ctx.restore();
}

function drawArrow(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number, width: number) {
  const angle = Math.atan2(y2 - y1, x2 - x1);
  const head = Math.max(width * 4, 12);
  // 主线止于箭头底边，避免粗线穿出箭尖。
  const shaftEndX = x2 - head * 0.8 * Math.cos(angle);
  const shaftEndY = y2 - head * 0.8 * Math.sin(angle);
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(shaftEndX, shaftEndY);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(x2 - head * Math.cos(angle - Math.PI / 7), y2 - head * Math.sin(angle - Math.PI / 7));
  ctx.lineTo(x2 - head * Math.cos(angle + Math.PI / 7), y2 - head * Math.sin(angle + Math.PI / 7));
  ctx.closePath();
  ctx.fill();
}

function drawMosaic(ctx: CanvasRenderingContext2D, a: MosaicAnnotation, frame: FrameSource, cache: MosaicCache) {
  let blocks = cache.get(a.id);
  if (!blocks) {
    const data = pixelateBlocks(frame.rgba, frame.width, frame.height, a.rect, a.block);
    if (data.cols === 0 || data.rows === 0) return;
    blocks = document.createElement('canvas');
    blocks.width = data.cols;
    blocks.height = data.rows;
    blocks.getContext('2d')!.putImageData(new ImageData(data.rgba, data.cols, data.rows), 0, 0);
    cache.set(a.id, blocks);
  }
  const x = Math.max(0, Math.floor(a.rect.x));
  const y = Math.max(0, Math.floor(a.rect.y));
  ctx.beginPath();
  ctx.rect(a.rect.x, a.rect.y, a.rect.width, a.rect.height);
  ctx.clip();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(blocks, x, y, blocks.width * a.block, blocks.height * a.block);
}

function drawNumber(ctx: CanvasRenderingContext2D, x: number, y: number, value: number, size: number, color: string) {
  const radius = numberRadius(size);
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, Math.PI * 2);
  ctx.fill();
  ctx.lineWidth = Math.max(1, radius * 0.12);
  ctx.strokeStyle = 'rgba(255,255,255,0.9)';
  ctx.stroke();
  ctx.fillStyle = isLight(color) ? '#1d1d1f' : '#ffffff';
  ctx.font = `600 ${Math.round(radius * 1.15)}px ${TEXT_FONT_FAMILY}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(String(value), x, y + radius * 0.04);
}

function drawText(ctx: CanvasRenderingContext2D, x: number, y: number, text: string, size: number) {
  const fontSize = textFontSize(size);
  ctx.font = `500 ${fontSize}px ${TEXT_FONT_FAMILY}`;
  ctx.textBaseline = 'top';
  ctx.textAlign = 'left';
  text.split('\n').forEach((line, index) => {
    ctx.fillText(line, x, y + index * fontSize * TEXT_LINE_HEIGHT);
  });
}

/** 浅色底时序号文字改用深色，保证可读。 */
function isLight(hex: string): boolean {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!m) return false;
  const [r, g, b] = [m[1], m[2], m[3]].map((v) => parseInt(v, 16));
  return r * 0.299 + g * 0.587 + b * 0.114 > 186;
}
