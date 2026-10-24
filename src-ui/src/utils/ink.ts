/**
 * 桌面屏幕注释（Screen Ink）绘图引擎与数据结构
 * 零第三方大型依赖，纯 Canvas 2D + PointerEvent 压感与二次贝塞尔平滑插值
 */

export type InkTool = 'pen' | 'highlighter' | 'laser' | 'line' | 'rect' | 'arrow' | 'eraser';

export interface InkPoint {
  x: number;
  y: number;
  pressure: number;
  timestamp: number;
}

export interface InkStroke {
  id: string;
  tool: InkTool;
  color: string;
  width: number;
  points: InkPoint[];
}

/** 历史记录状态管理（撤销/重做/清屏） */
export class InkHistoryManager {
  private undoStack: InkStroke[] = [];
  private redoStack: InkStroke[] = [];

  constructor(initialStrokes: InkStroke[] = []) {
    this.undoStack = [...initialStrokes];
  }

  get strokes(): InkStroke[] {
    return this.undoStack;
  }

  push(stroke: InkStroke): void {
    this.undoStack.push(stroke);
    this.redoStack = []; // 新输入清空重做栈
  }

  undo(): InkStroke | null {
    const stroke = this.undoStack.pop();
    if (stroke) {
      this.redoStack.push(stroke);
      return stroke;
    }
    return null;
  }

  redo(): InkStroke | null {
    const stroke = this.redoStack.pop();
    if (stroke) {
      this.undoStack.push(stroke);
      return stroke;
    }
    return null;
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
  }

  canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  canRedo(): boolean {
    return this.redoStack.length > 0;
  }
}

/** 激光笔 2 秒消隐点管理器 */
export class LaserPointerManager {
  private points: InkPoint[] = [];
  readonly lifetimeMs = 2000; // 2秒消隐规范

  addPoint(x: number, y: number): void {
    this.points.push({
      x,
      y,
      pressure: 1.0,
      timestamp: Date.now(),
    });
  }

  prune(now: number = Date.now()): void {
    const cutoff = now - this.lifetimeMs;
    this.points = this.points.filter((p) => p.timestamp >= cutoff);
  }

  clear(): void {
    this.points = [];
  }

  get activePoints(): InkPoint[] {
    return this.points;
  }

  hasPoints(): boolean {
    return this.points.length > 0;
  }

  render(ctx: CanvasRenderingContext2D, color: string = '#ef4444', now: number = Date.now()): void {
    this.prune(now);
    if (this.points.length === 0) return;

    ctx.save();
    for (let i = 0; i < this.points.length; i++) {
      const p = this.points[i];
      const age = now - p.timestamp;
      const alpha = Math.max(0, 1 - age / this.lifetimeMs);
      const radius = 6 * alpha + 2;

      // 外圈光晕
      ctx.beginPath();
      ctx.arc(p.x, p.y, radius + 3, 0, Math.PI * 2);
      ctx.fillStyle = color;
      ctx.globalAlpha = alpha * 0.35;
      ctx.fill();

      // 中心亮斑
      ctx.beginPath();
      ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
      ctx.fillStyle = '#ffffff';
      ctx.globalAlpha = alpha * 0.9;
      ctx.fill();
    }
    ctx.restore();
  }
}

/**
 * 绘制单个墨迹笔画
 */
export function renderStroke(ctx: CanvasRenderingContext2D, stroke: InkStroke): void {
  const { tool, color, width, points } = stroke;
  if (!points || points.length === 0) return;

  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';

  if (tool === 'eraser') {
    // 橡皮擦：采用 destination-out 擦除现有墨迹
    ctx.globalCompositeOperation = 'destination-out';
    ctx.lineWidth = width * 2.5;
    drawFreehandPoints(ctx, points, width * 2.5);
  } else if (tool === 'highlighter') {
    // 荧光笔：半透明高亮并适度加宽
    ctx.globalAlpha = 0.35;
    ctx.lineWidth = width * 2.2;
    drawFreehandPoints(ctx, points, width * 2.2);
  } else if (tool === 'pen') {
    // 普通画笔：支持压感动态粗细与二次贝塞尔平滑
    ctx.globalAlpha = 1.0;
    drawPressurePen(ctx, points, width);
  } else if (tool === 'line') {
    // 直线
    const start = points[0];
    const end = points[points.length - 1];
    ctx.beginPath();
    ctx.moveTo(start.x, start.y);
    ctx.lineTo(end.x, end.y);
    ctx.stroke();
  } else if (tool === 'rect') {
    // 矩形
    const start = points[0];
    const end = points[points.length - 1];
    const x = Math.min(start.x, end.x);
    const y = Math.min(start.y, end.y);
    const w = Math.abs(end.x - start.x);
    const h = Math.abs(end.y - start.y);
    ctx.beginPath();
    ctx.strokeRect(x, y, w, h);
  } else if (tool === 'arrow') {
    // 箭头
    const start = points[0];
    const end = points[points.length - 1];
    drawArrow(ctx, start.x, start.y, end.x, end.y, width);
  }

  ctx.restore();
}

/** 压感画笔平滑连续绘制 */
function drawPressurePen(ctx: CanvasRenderingContext2D, points: InkPoint[], baseWidth: number): void {
  if (points.length === 1) {
    const p = points[0];
    const r = (baseWidth * (0.5 + (p.pressure || 0.5) * 0.7)) / 2;
    ctx.beginPath();
    ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fill();
    return;
  }

  for (let i = 0; i < points.length - 1; i++) {
    const p1 = points[i];
    const p2 = points[i + 1];
    const pressure = (p1.pressure + p2.pressure) / 2 || 0.5;
    ctx.lineWidth = Math.max(1, baseWidth * (0.5 + pressure * 0.7));

    ctx.beginPath();
    ctx.moveTo(p1.x, p1.y);
    if (i < points.length - 2) {
      const p3 = points[i + 2];
      const midX = (p2.x + p3.x) / 2;
      const midY = (p2.y + p3.y) / 2;
      ctx.quadraticCurveTo(p2.x, p2.y, midX, midY);
    } else {
      ctx.lineTo(p2.x, p2.y);
    }
    ctx.stroke();
  }
}

/** 连续点拟合绘制 */
function drawFreehandPoints(ctx: CanvasRenderingContext2D, points: InkPoint[], _width: number): void {
  if (points.length === 1) {
    ctx.beginPath();
    ctx.arc(points[0].x, points[0].y, ctx.lineWidth / 2, 0, Math.PI * 2);
    ctx.fill();
    return;
  }
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    ctx.lineTo(points[i].x, points[i].y);
  }
  ctx.stroke();
}

/** 绘制带方向箭头的直线 */
function drawArrow(
  ctx: CanvasRenderingContext2D,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  lineWidth: number
): void {
  const headlen = Math.max(lineWidth * 3.5, 14); // 箭头翼长
  const angle = Math.atan2(toY - fromY, toX - fromX);

  // 1. 绘制主线
  ctx.beginPath();
  ctx.moveTo(fromX, fromY);
  ctx.lineTo(toX, toY);
  ctx.stroke();

  // 2. 绘制实心箭头三角形
  ctx.beginPath();
  ctx.moveTo(toX, toY);
  ctx.lineTo(
    toX - headlen * Math.cos(angle - Math.PI / 6),
    toY - headlen * Math.sin(angle - Math.PI / 6)
  );
  ctx.lineTo(
    toX - headlen * Math.cos(angle + Math.PI / 6),
    toY - headlen * Math.sin(angle + Math.PI / 6)
  );
  ctx.closePath();
  ctx.fill();
}
