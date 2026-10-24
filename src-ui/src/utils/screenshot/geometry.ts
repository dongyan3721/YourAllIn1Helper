/**
 * 截图选区几何（纯函数，便于单元测试）。
 *
 * 所有矩形均使用「帧像素」坐标：与后端冻结帧的物理像素一一对应，原点为被截显示器左上角。
 * 选区的语义与后端 `app_core::SelectionBox` 一致（x/y 为左上角，width/height 为非负整数），
 * 交互拖动需要零延迟，因此在前端计算；后端导出时再用 `normalize_crop` 校验并裁剪。
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

/** 八个调整手柄（四角 + 四边），命名与后端 `ResizeHandle` 一致。 */
export type ResizeHandle =
  | 'top_left'
  | 'top'
  | 'top_right'
  | 'right'
  | 'bottom_right'
  | 'bottom'
  | 'bottom_left'
  | 'left';

export const RESIZE_HANDLES: ResizeHandle[] = [
  'top_left',
  'top',
  'top_right',
  'right',
  'bottom_right',
  'bottom',
  'bottom_left',
  'left',
];

/** 由两个任意拖拽端点生成规范化矩形（宽高非负，坐标取整）。 */
export function rectFromPoints(a: Point, b: Point): Rect {
  const left = Math.round(Math.min(a.x, b.x));
  const top = Math.round(Math.min(a.y, b.y));
  const right = Math.round(Math.max(a.x, b.x));
  const bottom = Math.round(Math.max(a.y, b.y));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** 将矩形完整限制在边界内：先收缩尺寸，再平移到边界内（与后端 clamp_to_bounds 一致）。 */
export function clampRectInside(rect: Rect, bounds: Rect): Rect {
  const width = Math.min(Math.max(0, rect.width), bounds.width);
  const height = Math.min(Math.max(0, rect.height), bounds.height);
  const x = Math.min(Math.max(rect.x, bounds.x), bounds.x + bounds.width - width);
  const y = Math.min(Math.max(rect.y, bounds.y), bounds.y + bounds.height - height);
  return { x, y, width, height };
}

/** 求矩形与边界的交集；无交集时返回 null。用于裁掉超出屏幕的拖拽部分。 */
export function intersectRect(rect: Rect, bounds: Rect): Rect | null {
  const left = Math.max(rect.x, bounds.x);
  const top = Math.max(rect.y, bounds.y);
  const right = Math.min(rect.x + rect.width, bounds.x + bounds.width);
  const bottom = Math.min(rect.y + rect.height, bounds.y + bounds.height);
  if (right <= left || bottom <= top) return null;
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** 平移选区并保持在边界内（方向键 1px、Space 拖移、拖动选区内部）。 */
export function translateRect(rect: Rect, dx: number, dy: number, bounds: Rect): Rect {
  return clampRectInside({ ...rect, x: rect.x + dx, y: rect.y + dy }, bounds);
}

/**
 * 按手柄拖动调整尺寸。拖过对边时自动翻转（规范化），结果裁到边界内且至少 1px。
 * 返回新的矩形与翻转后实际对应的手柄，调用方据此继续拖动。
 */
export function resizeWithHandle(
  rect: Rect,
  handle: ResizeHandle,
  dx: number,
  dy: number,
  bounds: Rect
): { rect: Rect; handle: ResizeHandle } {
  let left = rect.x;
  let top = rect.y;
  let right = rect.x + rect.width;
  let bottom = rect.y + rect.height;
  if (handle === 'top_left' || handle === 'left' || handle === 'bottom_left') left += dx;
  if (handle === 'top_right' || handle === 'right' || handle === 'bottom_right') right += dx;
  if (handle === 'top_left' || handle === 'top' || handle === 'top_right') top += dy;
  if (handle === 'bottom_left' || handle === 'bottom' || handle === 'bottom_right') bottom += dy;

  const flipX = right < left;
  const flipY = bottom < top;
  const raw = rectFromPoints({ x: left, y: top }, { x: right, y: bottom });
  // 逐边裁到边界内并保证至少 1px，边重合时也不丢失另一维尺寸。
  const l = clamp(raw.x, bounds.x, bounds.x + bounds.width - 1);
  const t = clamp(raw.y, bounds.y, bounds.y + bounds.height - 1);
  const r = clamp(raw.x + raw.width, l + 1, bounds.x + bounds.width);
  const b = clamp(raw.y + raw.height, t + 1, bounds.y + bounds.height);
  return {
    rect: { x: l, y: t, width: r - l, height: b - t },
    handle: flipHandle(handle, flipX, flipY),
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function flipHandle(handle: ResizeHandle, flipX: boolean, flipY: boolean): ResizeHandle {
  let h = handle;
  if (flipX) {
    h = (
      {
        top_left: 'top_right',
        top_right: 'top_left',
        left: 'right',
        right: 'left',
        bottom_left: 'bottom_right',
        bottom_right: 'bottom_left',
        top: 'top',
        bottom: 'bottom',
      } as Record<ResizeHandle, ResizeHandle>
    )[h];
  }
  if (flipY) {
    h = (
      {
        top_left: 'bottom_left',
        bottom_left: 'top_left',
        top: 'bottom',
        bottom: 'top',
        top_right: 'bottom_right',
        bottom_right: 'top_right',
        left: 'left',
        right: 'right',
      } as Record<ResizeHandle, ResizeHandle>
    )[h];
  }
  return h;
}

export type ArrowKey = 'ArrowUp' | 'ArrowDown' | 'ArrowLeft' | 'ArrowRight';

/**
 * 键盘微调：方向键平移 1px；Shift + 方向键微调宽高 1px（→/↓ 增大，←/↑ 减小，最小 1px）。
 */
export function nudgeRect(rect: Rect, key: ArrowKey, resize: boolean, bounds: Rect, step = 1): Rect {
  const dx = key === 'ArrowLeft' ? -step : key === 'ArrowRight' ? step : 0;
  const dy = key === 'ArrowUp' ? -step : key === 'ArrowDown' ? step : 0;
  if (!resize) return translateRect(rect, dx, dy, bounds);
  const width = Math.max(1, Math.min(rect.width + dx, bounds.x + bounds.width - rect.x));
  const height = Math.max(1, Math.min(rect.height + dy, bounds.y + bounds.height - rect.y));
  return { ...rect, width, height };
}

/** 点是否在矩形内（右/下边界开区间，与后端 contains_point 一致）。 */
export function containsPoint(rect: Rect, p: Point): boolean {
  return p.x >= rect.x && p.x < rect.x + rect.width && p.y >= rect.y && p.y < rect.y + rect.height;
}

/** CSS 像素 ↔ 帧像素的比例（遮罩窗口可能被系统微调尺寸，因此 x/y 分开计算）。 */
export interface FrameScale {
  sx: number;
  sy: number;
}

export function computeFrameScale(
  frameWidth: number,
  frameHeight: number,
  viewWidth: number,
  viewHeight: number
): FrameScale {
  return {
    sx: viewWidth > 0 ? frameWidth / viewWidth : 1,
    sy: viewHeight > 0 ? frameHeight / viewHeight : 1,
  };
}

export function cssToFrame(p: Point, scale: FrameScale): Point {
  return { x: Math.round(p.x * scale.sx), y: Math.round(p.y * scale.sy) };
}

export function frameRectToCss(rect: Rect, scale: FrameScale): Rect {
  return {
    x: rect.x / scale.sx,
    y: rect.y / scale.sy,
    width: rect.width / scale.sx,
    height: rect.height / scale.sy,
  };
}

/** 判断拖拽距离是否足以视为「拖选」，否则视为单击（采用智能区域）。 */
export function isDrag(a: Point, b: Point, threshold: number): boolean {
  return Math.abs(a.x - b.x) >= threshold || Math.abs(a.y - b.y) >= threshold;
}

/**
 * 计算工具栏位置（CSS 像素）：优先选区下方右对齐，空间不足放上方，再不足放选区内底部；
 * 结果始终限制在视口内。
 */
export function placeToolbar(
  selection: Rect,
  toolbar: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 8
): Point {
  const maxX = Math.max(0, viewport.width - toolbar.width);
  const x = Math.min(Math.max(0, selection.x + selection.width - toolbar.width), maxX);
  const below = selection.y + selection.height + gap;
  if (below + toolbar.height <= viewport.height) return { x, y: below };
  const above = selection.y - gap - toolbar.height;
  if (above >= 0) return { x, y: above };
  const inside = Math.max(0, selection.y + selection.height - gap - toolbar.height);
  return { x, y: Math.min(inside, Math.max(0, viewport.height - toolbar.height)) };
}
