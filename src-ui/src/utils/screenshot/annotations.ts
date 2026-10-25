/**
 * 截图标注数据模型与撤销/重做历史（纯逻辑，不依赖 DOM）。
 *
 * 坐标为帧像素；线宽、字号等以「逻辑像素 × 设备像素比」换算到帧像素后保存，
 * 保证 Retina 屏上的标注与屏幕上看到的粗细一致，导出时无需再缩放。
 */
import type { Point, Rect } from './geometry.ts';

export type AnnotationTool = 'rect' | 'arrow' | 'mosaic' | 'number' | 'text';

export const ANNOTATION_TOOLS: AnnotationTool[] = ['rect', 'arrow', 'mosaic', 'number', 'text'];

/** 调色板（第一项与后端默认颜色一致）。 */
export const ANNOTATION_COLORS = ['#ff3b30', '#ff9500', '#ffcc00', '#34c759', '#007aff', '#af52de', '#ffffff', '#1d1d1f'];

/** 线宽档位（逻辑像素）。 */
export const STROKE_WIDTHS = [2, 4, 8];

interface AnnotationBase {
  id: string;
  color: string;
  /** 帧像素线宽 / 基准尺寸。 */
  size: number;
}

export interface RectAnnotation extends AnnotationBase {
  kind: 'rect';
  rect: Rect;
}

export interface ArrowAnnotation extends AnnotationBase {
  kind: 'arrow';
  from: Point;
  to: Point;
}

export interface MosaicAnnotation extends AnnotationBase {
  kind: 'mosaic';
  rect: Rect;
  /** 马赛克块边长（帧像素）。 */
  block: number;
}

export interface NumberAnnotation extends AnnotationBase {
  kind: 'number';
  at: Point;
  value: number;
}

export interface TextAnnotation extends AnnotationBase {
  kind: 'text';
  at: Point;
  text: string;
}

export type Annotation =
  | RectAnnotation
  | ArrowAnnotation
  | MosaicAnnotation
  | NumberAnnotation
  | TextAnnotation;

let idCounter = 0;
/** 会话内唯一 ID。 */
export function nextAnnotationId(): string {
  idCounter += 1;
  return `a${Date.now().toString(36)}${idCounter}`;
}

/** 由逻辑线宽换算帧像素尺寸。 */
export function toFrameSize(logicalWidth: number, devicePixelRatio: number): number {
  return Math.max(1, Math.round(logicalWidth * devicePixelRatio));
}

/** 马赛克块大小：随线宽档位放大，至少 6 帧像素。 */
export function mosaicBlockSize(frameSize: number): number {
  return Math.max(6, frameSize * 3);
}

/** 文字字号（帧像素）。 */
export function textFontSize(frameSize: number): number {
  return Math.max(12, Math.round(frameSize * 3.5 + 10));
}

/** 序号圆半径（帧像素）。 */
export function numberRadius(frameSize: number): number {
  return Math.max(10, Math.round(frameSize * 2 + 8));
}

/** 下一个序号：已有序号最大值 + 1（撤销后自动回退）。 */
export function nextNumberValue(items: readonly Annotation[]): number {
  return items.reduce((max, a) => (a.kind === 'number' ? Math.max(max, a.value) : max), 0) + 1;
}

/** 过小的拖拽结果（误触）不入历史。 */
export function isMeaningful(a: Annotation, minSize = 3): boolean {
  switch (a.kind) {
    case 'rect':
    case 'mosaic':
      return a.rect.width >= minSize && a.rect.height >= minSize;
    case 'arrow':
      return Math.hypot(a.to.x - a.from.x, a.to.y - a.from.y) >= minSize;
    case 'text':
      return a.text.trim().length > 0;
    case 'number':
      return true;
  }
}

/**
 * 快照式编辑历史：每次变更保存完整列表，撤销/重做只切换快照，不会出现局部状态不一致。
 * 标注数量有限（通常几十个），快照开销可以忽略。
 */
export class EditHistory {
  private past: Annotation[][] = [];
  private future: Annotation[][] = [];
  private present: Annotation[] = [];
  private readonly limit: number;

  constructor(limit = 200) {
    this.limit = limit;
  }

  get items(): readonly Annotation[] {
    return this.present;
  }

  get canUndo(): boolean {
    return this.past.length > 0;
  }

  get canRedo(): boolean {
    return this.future.length > 0;
  }

  private commit(next: Annotation[]): void {
    this.past.push(this.present);
    if (this.past.length > this.limit) this.past.shift();
    this.present = next;
    this.future = [];
  }

  add(annotation: Annotation): void {
    this.commit([...this.present, annotation]);
  }

  /** 替换同 ID 标注（如编辑已有文字）；ID 不存在时不产生历史记录。 */
  update(annotation: Annotation): boolean {
    const index = this.present.findIndex((a) => a.id === annotation.id);
    if (index < 0) return false;
    const next = [...this.present];
    next[index] = annotation;
    this.commit(next);
    return true;
  }

  remove(id: string): boolean {
    if (!this.present.some((a) => a.id === id)) return false;
    this.commit(this.present.filter((a) => a.id !== id));
    return true;
  }

  clear(): void {
    if (this.present.length === 0) return;
    this.commit([]);
  }

  undo(): boolean {
    const previous = this.past.pop();
    if (!previous) return false;
    this.future.push(this.present);
    this.present = previous;
    return true;
  }

  redo(): boolean {
    const next = this.future.pop();
    if (!next) return false;
    this.past.push(this.present);
    this.present = next;
    return true;
  }
}
