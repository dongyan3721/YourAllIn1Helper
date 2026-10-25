<template>
  <div
    ref="rootEl"
    class="capture-root"
    :class="cursorClass"
    tabindex="0"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @wheel.prevent="onWheel"
    @contextmenu.prevent="onContextMenu"
    @dblclick="onDoubleClick"
  >
    <!-- 冻结帧底图与遮罩/标注层；两层均为帧像素分辨率，CSS 拉伸到整个窗口 -->
    <canvas ref="baseCanvas" class="layer"></canvas>
    <canvas ref="uiCanvas" class="layer"></canvas>

    <!-- 加载或冻结帧读取失败：如实显示并允许取消 -->
    <div v-if="loadError" class="capture-error" @pointerdown.stop>
      <strong>{{ t('screenshot.overlay.loadFailed') }}</strong>
      <span>{{ loadError }}</span>
      <button class="tool-btn text-btn" @click="cancelCapture">{{ t('screenshot.overlay.cancel') }}</button>
    </div>

    <!-- 尚未选区时的操作提示 -->
    <div v-if="ready && !selection" class="capture-hint">{{ idleHint }}</div>

    <!-- 智能区域名称 -->
    <div v-if="ready && !selection && hoverCss && hoverRegion?.label" class="region-label" :style="labelStyle(hoverCss)">
      {{ hoverRegion.label }}
    </div>

    <!-- 选区尺寸（帧像素，即导出图片的真实像素） -->
    <div v-if="selectionCss" class="size-badge" :style="labelStyle(selectionCss)">
      {{ selection!.width }} × {{ selection!.height }}
    </div>

    <!-- 八个调整手柄 -->
    <template v-if="selectionCss && !exporting">
      <div
        v-for="h in RESIZE_HANDLES"
        :key="h"
        class="handle"
        :class="`handle-${h}`"
        :style="handleStyle(h)"
        @pointerdown.stop.prevent="startResize($event, h)"
      ></div>
    </template>

    <!-- 文字标注输入框 -->
    <textarea
      v-if="textEditor"
      ref="textEl"
      v-model="textEditor.value"
      class="text-editor"
      :style="textEditorStyle"
      :placeholder="t('screenshot.overlay.textPlaceholder')"
      @pointerdown.stop
      @keydown.stop="onTextKeydown"
      @blur="commitText"
    ></textarea>

    <!-- 标注工具栏：与画布同属一个 WebView，z-index 位于画布之上，并阻止事件穿到画布 -->
    <div
      v-if="selectionCss && !dragging"
      ref="toolbarEl"
      class="capture-toolbar"
      :style="toolbarStyle"
      @pointerdown.stop
      @pointerup.stop
      @dblclick.stop
      @wheel.stop
      @contextmenu.stop.prevent
    >
      <div class="toolbar-row">
        <button
          v-for="(tl, index) in ANNOTATION_TOOLS"
          :key="tl"
          class="tool-btn"
          :class="{ active: tool === tl }"
          :title="`${t('screenshot.overlay.tools.' + tl)} (${index + 1})`"
          :disabled="exporting"
          @click="toggleTool(tl)"
        >
          <Icons v-if="tl === 'rect' || tl === 'arrow' || tl === 'text'" :name="tl" :size="16" />
          <svg v-else-if="tl === 'mosaic'" width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
            <rect x="2" y="2" width="4" height="4" /><rect x="10" y="2" width="4" height="4" opacity=".55" />
            <rect x="6" y="6" width="4" height="4" opacity=".55" /><rect x="2" y="10" width="4" height="4" opacity=".55" />
            <rect x="10" y="10" width="4" height="4" />
          </svg>
          <svg v-else width="16" height="16" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5" />
            <text x="8" y="11.2" text-anchor="middle" font-size="8.5" font-weight="700" fill="currentColor">1</text>
          </svg>
        </button>
        <span class="sep"></span>
        <button class="tool-btn" :title="t('screenshot.overlay.undo')" :disabled="!canUndo || exporting" @click="undo">
          <Icons name="undo" :size="16" />
        </button>
        <button class="tool-btn" :title="t('screenshot.overlay.redo')" :disabled="!canRedo || exporting" @click="redo">
          <Icons name="redo" :size="16" />
        </button>
        <span class="sep"></span>
        <button class="tool-btn" :title="t('screenshot.overlay.pin') + ' (F3)'" :disabled="exporting" @click="doExport('pin')">
          <Icons name="pin" :size="16" />
        </button>
        <button class="tool-btn" :title="t('screenshot.overlay.save') + ' (⌘/Ctrl+S)'" :disabled="exporting" @click="doExport('save')">
          <Icons name="download" :size="16" />
        </button>
        <button class="tool-btn" :title="t('screenshot.overlay.cancel') + ' (Esc)'" :disabled="exporting" @click="cancelCapture">
          <Icons name="close" :size="16" />
        </button>
        <button class="tool-btn primary" :title="t('screenshot.overlay.copy') + ' (Enter)'" :disabled="exporting" @click="doExport('copy')">
          <Icons name="check" :size="16" />
        </button>
      </div>
      <div v-if="tool" class="toolbar-row sub-row">
        <template v-if="tool !== 'mosaic'">
          <button
            v-for="c in ANNOTATION_COLORS"
            :key="c"
            class="color-dot"
            :class="{ active: color === c }"
            :style="{ backgroundColor: c }"
            :title="c"
            @click="color = c"
          ></button>
          <span class="sep"></span>
        </template>
        <button
          v-for="w in STROKE_WIDTHS"
          :key="w"
          class="width-btn"
          :class="{ active: strokeWidth === w }"
          :title="t('screenshot.overlay.widthLabel', { width: w })"
          @click="strokeWidth = w"
        >
          <span class="width-dot" :style="{ width: `${4 + w}px`, height: `${4 + w}px` }"></span>
        </button>
      </div>
    </div>

    <!-- 放大镜与取色 -->
    <div v-if="showMagnifier" class="magnifier" :style="magnifierStyle">
      <canvas ref="magCanvas" class="mag-canvas"></canvas>
      <div class="mag-info">
        <div>{{ probe.x }}, {{ probe.y }}</div>
        <div v-if="probeColor" class="mag-color">
          <span class="swatch" :style="{ backgroundColor: probeHex }"></span>
          <span>{{ probeHex }}</span>
        </div>
        <div v-if="probeColor">{{ probeRgb }}</div>
        <div class="mag-hint">{{ t('screenshot.overlay.colorHint') }}</div>
      </div>
    </div>

    <div v-if="toast" class="capture-toast" :class="toast.kind">{{ toast.text }}</div>
  </div>
</template>

<script setup lang="ts">
/**
 * 截图遮罩（capture-overlay）：冻结帧显示、手动/智能选区、标注编辑、放大镜取色与导出。
 *
 * 坐标统一为帧像素（与后端冻结帧物理像素一致）；DOM 元素位置由帧像素按 CSS 比例换算。
 * 冻结帧以二进制 IPC 获取后直接构造 ImageData，标注层渲染为选区大小的 PNG 交回后端合成。
 */
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, shallowRef } from 'vue';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';
import {
  exportSelection,
  fetchFrozenFrame,
  screenshotApi,
  type DetectedRegion,
  type ExportAction,
  type ScreenshotSessionInfo,
} from '../api/screenshot';
import {
  RESIZE_HANDLES,
  computeFrameScale,
  containsPoint,
  cssToFrame,
  frameRectToCss,
  intersectRect,
  isDrag,
  nudgeRect,
  placeToolbar,
  rectFromPoints,
  resizeWithHandle,
  translateRect,
  type ArrowKey,
  type Point,
  type Rect,
  type ResizeHandle,
} from '../utils/screenshot/geometry';
import {
  ANNOTATION_COLORS,
  ANNOTATION_TOOLS,
  STROKE_WIDTHS,
  EditHistory,
  isMeaningful,
  mosaicBlockSize,
  nextAnnotationId,
  nextNumberValue,
  textFontSize,
  toFrameSize,
  type Annotation,
  type AnnotationTool,
} from '../utils/screenshot/annotations';
import { readPixel, toHex, toRgbText } from '../utils/screenshot/color';
import { renderAnnotations, TEXT_FONT_FAMILY, TEXT_LINE_HEIGHT, type FrameSource, type MosaicCache } from '../utils/screenshot/render';

type DragState =
  | { kind: 'create'; start: Point; moved: boolean }
  | { kind: 'move'; start: Point; origin: Rect }
  | { kind: 'resize'; handle: ResizeHandle; last: Point; rect: Rect }
  | { kind: 'draw'; start: Point };

/** 放大镜显示的像素格数与每格 CSS 尺寸。 */
const MAG_CELLS = 15;
const MAG_CELL_PX = 8;
const MAG_SIZE = MAG_CELLS * MAG_CELL_PX;
/** 拖动超过该 CSS 距离才算拖选，否则视为单击采用智能区域。 */
const DRAG_THRESHOLD_CSS = 4;

const rootEl = ref<HTMLDivElement | null>(null);
const baseCanvas = ref<HTMLCanvasElement | null>(null);
const uiCanvas = ref<HTMLCanvasElement | null>(null);
const magCanvas = ref<HTMLCanvasElement | null>(null);
const toolbarEl = ref<HTMLDivElement | null>(null);
const textEl = ref<HTMLTextAreaElement | null>(null);

const session = shallowRef<ScreenshotSessionInfo | null>(null);
let frame: FrameSource | null = null;
const ready = ref(false);
const loadError = ref('');
const exporting = ref(false);
const toast = ref<{ text: string; kind: 'info' | 'error' } | null>(null);

const viewport = reactive({ width: window.innerWidth, height: window.innerHeight });
const scale = computed(() =>
  computeFrameScale(session.value?.frame_width ?? 1, session.value?.frame_height ?? 1, viewport.width, viewport.height)
);
const bounds = computed<Rect>(() => ({
  x: 0,
  y: 0,
  width: session.value?.frame_width ?? 0,
  height: session.value?.frame_height ?? 0,
}));

const selection = ref<Rect | null>(null);
const hoverRegion = shallowRef<DetectedRegion | null>(null);
const dragging = ref<DragState | null>(null);
const spaceHeld = ref(false);
const tool = ref<AnnotationTool | null>(null);
const color = ref(ANNOTATION_COLORS[0]);
const strokeWidth = ref(STROKE_WIDTHS[1]);
const history = new EditHistory();
const historyVersion = ref(0);
const draft = shallowRef<Annotation | null>(null);
const textEditor = ref<{ at: Point; value: string; size: number; color: string } | null>(null);
const probe = ref<Point>({ x: 0, y: 0 });
const mosaicCache: MosaicCache = new Map();
const toolbarSize = reactive({ width: 380, height: 44 });

let levelSteps = 0;
let levelAnchorCss: Point | null = null;
let resolving = false;
let pendingResolve: Point | null = null;
let renderQueued = false;
let toastTimer: ReturnType<typeof setTimeout> | undefined;

// EditHistory 不是响应式对象，读取 historyVersion 让按钮状态随每次变更刷新。
const canUndo = computed(() => historyVersion.value >= 0 && history.canUndo);
const canRedo = computed(() => historyVersion.value >= 0 && history.canRedo);
const frameSize = computed(() => toFrameSize(strokeWidth.value, session.value?.device_pixel_ratio ?? 1));

const selectionCss = computed(() => (selection.value ? frameRectToCss(selection.value, scale.value) : null));
const hoverCss = computed(() => (hoverRegion.value ? frameRectToCss(hoverRegion.value, scale.value) : null));

const idleHint = computed(() =>
  session.value?.settings.smart_regions && session.value.window_region_count > 0
    ? t('screenshot.overlay.hintSmart')
    : t('screenshot.overlay.hintManual')
);

const cursorClass = computed(() => {
  if (spaceHeld.value && selection.value) return 'cursor-grab';
  if (dragging.value?.kind === 'move') return 'cursor-grabbing';
  if (selection.value && !tool.value && containsPoint(selection.value, probe.value)) return 'cursor-move';
  return 'cursor-cross';
});

const showMagnifier = computed(
  () =>
    ready.value &&
    !!session.value?.settings.magnifier &&
    !textEditor.value &&
    !exporting.value &&
    (!selection.value || dragging.value?.kind === 'create' || dragging.value?.kind === 'resize')
);

const probeColor = computed(() =>
  frame ? readPixel(frame.rgba, frame.width, frame.height, probe.value.x, probe.value.y) : null
);
const probeHex = computed(() => (probeColor.value ? toHex(probeColor.value) : ''));
const probeRgb = computed(() => (probeColor.value ? toRgbText(probeColor.value) : ''));

const magnifierStyle = computed(() => {
  const p = { x: probe.value.x / scale.value.sx, y: probe.value.y / scale.value.sy };
  const w = MAG_SIZE + 2;
  const h = MAG_SIZE + 78;
  const x = p.x + 20 + w > viewport.width ? p.x - 20 - w : p.x + 20;
  const y = p.y + 20 + h > viewport.height ? p.y - 20 - h : p.y + 20;
  return { left: `${Math.max(0, x)}px`, top: `${Math.max(0, y)}px` };
});

const toolbarStyle = computed(() => {
  if (!selectionCss.value) return {};
  const p = placeToolbar(selectionCss.value, toolbarSize, viewport);
  return { left: `${p.x}px`, top: `${p.y}px` };
});

const textEditorStyle = computed(() => {
  if (!textEditor.value) return {};
  const fontCss = textFontSize(textEditor.value.size) / scale.value.sy;
  return {
    left: `${textEditor.value.at.x / scale.value.sx}px`,
    top: `${textEditor.value.at.y / scale.value.sy}px`,
    fontSize: `${fontCss}px`,
    lineHeight: String(TEXT_LINE_HEIGHT),
    color: textEditor.value.color,
    fontFamily: TEXT_FONT_FAMILY,
  };
});

function labelStyle(rect: Rect) {
  const top = rect.y >= 26 ? rect.y - 24 : rect.y + 4;
  return { left: `${Math.max(0, rect.x)}px`, top: `${top}px` };
}

function handleStyle(h: ResizeHandle) {
  const r = selectionCss.value!;
  const cx = h.includes('left') ? r.x : h.includes('right') ? r.x + r.width : r.x + r.width / 2;
  const cy = h.startsWith('top') ? r.y : h.startsWith('bottom') ? r.y + r.height : r.y + r.height / 2;
  return { left: `${cx - 4}px`, top: `${cy - 4}px` };
}

// ───────────────────────────── 渲染 ─────────────────────────────

function requestRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    renderUi();
    renderMagnifier();
  });
}

function renderUi() {
  const canvas = uiCanvas.value;
  if (!canvas || !frame) return;
  const ctx = canvas.getContext('2d')!;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  const sel = selection.value;
  const lineWidth = Math.max(1, Math.round(scale.value.sx));
  ctx.fillStyle = 'rgba(0, 0, 0, 0.42)';
  ctx.fillRect(0, 0, width, height);
  const focus = sel ?? hoverRegion.value;
  if (focus) {
    // 清除遮罩露出冻结帧原样像素。
    ctx.clearRect(focus.x, focus.y, focus.width, focus.height);
  }
  if (sel) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(sel.x, sel.y, sel.width, sel.height);
    ctx.clip();
    renderAnnotations(ctx, history.items, frame, mosaicCache);
    if (draft.value) renderAnnotations(ctx, [draft.value], frame, new Map());
    ctx.restore();
  }
  if (focus) {
    ctx.save();
    ctx.strokeStyle = sel ? '#0a84ff' : 'rgba(10, 132, 255, 0.95)';
    ctx.lineWidth = sel ? lineWidth : lineWidth * 2;
    ctx.strokeRect(focus.x + ctx.lineWidth / 2, focus.y + ctx.lineWidth / 2, focus.width - ctx.lineWidth, focus.height - ctx.lineWidth);
    ctx.restore();
  }
}

function renderMagnifier() {
  const canvas = magCanvas.value;
  const base = baseCanvas.value;
  if (!canvas || !base || !showMagnifier.value) return;
  const dpr = window.devicePixelRatio || 1;
  const size = Math.round(MAG_SIZE * dpr);
  if (canvas.width !== size) {
    canvas.width = size;
    canvas.height = size;
  }
  const ctx = canvas.getContext('2d')!;
  const half = Math.floor(MAG_CELLS / 2);
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, size, size);
  ctx.drawImage(base, probe.value.x - half, probe.value.y - half, MAG_CELLS, MAG_CELLS, 0, 0, size, size);
  const cell = size / MAG_CELLS;
  ctx.strokeStyle = 'rgba(10, 132, 255, 0.9)';
  ctx.lineWidth = Math.max(1, dpr);
  ctx.strokeRect(half * cell, half * cell, cell, cell);
}

// ───────────────────────────── 坐标与智能区域 ─────────────────────────────

function framePoint(e: { clientX: number; clientY: number }): Point {
  const p = cssToFrame({ x: e.clientX, y: e.clientY }, scale.value);
  return {
    x: Math.min(Math.max(0, p.x), bounds.value.width - 1),
    y: Math.min(Math.max(0, p.y), bounds.value.height - 1),
  };
}

function scheduleResolve(p: Point) {
  if (!session.value?.settings.smart_regions) return;
  pendingResolve = p;
  if (!resolving) void pumpResolve();
}

/** 串行请求光标处区域：同一时刻最多一个请求，期间只保留最新坐标。 */
async function pumpResolve() {
  resolving = true;
  while (pendingResolve && session.value) {
    const pt = pendingResolve;
    pendingResolve = null;
    try {
      const region = await screenshotApi.resolveRegion(session.value.session_id, pt.x, pt.y, levelSteps);
      if (!selection.value && !dragging.value) {
        hoverRegion.value = region;
        requestRender();
      }
    } catch {
      hoverRegion.value = null;
      break;
    }
  }
  resolving = false;
}

// ───────────────────────────── 指针交互 ─────────────────────────────

function onPointerDown(e: PointerEvent) {
  if (!ready.value || exporting.value || e.button !== 0) return;
  if (textEditor.value) {
    commitText();
    return;
  }
  const p = framePoint(e);
  probe.value = p;
  const sel = selection.value;
  rootEl.value?.setPointerCapture(e.pointerId);
  if (sel && spaceHeld.value) {
    dragging.value = { kind: 'move', start: p, origin: sel };
  } else if (sel && tool.value && containsPoint(sel, p)) {
    beginAnnotation(p);
  } else if (sel && !tool.value && containsPoint(sel, p)) {
    dragging.value = { kind: 'move', start: p, origin: sel };
  } else if (!sel || history.items.length === 0) {
    // 已有标注时点击选区外不重新选区，避免误操作丢失编辑结果。
    dragging.value = { kind: 'create', start: p, moved: false };
  }
}

function startResize(e: PointerEvent, handle: ResizeHandle) {
  if (!selection.value || exporting.value) return;
  rootEl.value?.setPointerCapture(e.pointerId);
  dragging.value = { kind: 'resize', handle, last: framePoint(e), rect: selection.value };
}

function onPointerMove(e: PointerEvent) {
  if (!ready.value) return;
  const p = framePoint(e);
  probe.value = p;
  const d = dragging.value;
  if (!d) {
    if (!selection.value) {
      if (levelAnchorCss && Math.hypot(e.clientX - levelAnchorCss.x, e.clientY - levelAnchorCss.y) > 24) {
        levelSteps = 0;
        levelAnchorCss = null;
      }
      scheduleResolve(p);
    }
    requestRender();
    return;
  }
  const b = bounds.value;
  if (d.kind === 'create') {
    if (!session.value?.settings.manual_selection) return;
    if (!d.moved && isDrag(d.start, p, DRAG_THRESHOLD_CSS * scale.value.sx)) d.moved = true;
    if (d.moved) {
      hoverRegion.value = null;
      selection.value = intersectRect(rectFromPoints(d.start, { x: p.x + 1, y: p.y + 1 }), b);
    }
  } else if (d.kind === 'move') {
    selection.value = translateRect(d.origin, p.x - d.start.x, p.y - d.start.y, b);
  } else if (d.kind === 'resize') {
    const next = resizeWithHandle(d.rect, d.handle, p.x - d.last.x, p.y - d.last.y, b);
    d.rect = next.rect;
    d.handle = next.handle;
    d.last = p;
    selection.value = next.rect;
  } else if (d.kind === 'draw' && draft.value && selection.value) {
    draft.value = updateDraft(draft.value, d.start, clampToSelection(p));
  }
  requestRender();
}

function onPointerUp(e: PointerEvent) {
  const d = dragging.value;
  if (!d) return;
  if (rootEl.value?.hasPointerCapture(e.pointerId)) rootEl.value.releasePointerCapture(e.pointerId);
  if (d.kind === 'create' && !d.moved) {
    // 单击：采用当前智能区域（窗口或整屏）。
    if (hoverRegion.value) {
      selection.value = { x: hoverRegion.value.x, y: hoverRegion.value.y, width: hoverRegion.value.width, height: hoverRegion.value.height };
    }
  } else if (d.kind === 'create' && selection.value && (selection.value.width < 2 || selection.value.height < 2)) {
    selection.value = null;
  } else if (d.kind === 'draw' && draft.value) {
    if (isMeaningful(draft.value)) {
      history.add(draft.value);
      historyVersion.value++;
    }
    draft.value = null;
  }
  if (selection.value) hoverRegion.value = null;
  dragging.value = null;
  void measureToolbar();
  requestRender();
}

function onWheel(e: WheelEvent) {
  if (selection.value || !session.value?.settings.wheel_expand || !session.value.settings.smart_regions) return;
  levelSteps = Math.min(8, Math.max(0, levelSteps + (e.deltaY < 0 ? 1 : -1)));
  levelAnchorCss = { x: e.clientX, y: e.clientY };
  scheduleResolve(framePoint(e));
}

function onContextMenu() {
  if (textEditor.value) {
    textEditor.value = null;
    return;
  }
  if (!selection.value) {
    void cancelCapture();
  } else if (history.items.length === 0) {
    selection.value = null;
    tool.value = null;
    scheduleResolve(probe.value);
    requestRender();
  } else {
    showToast(t('screenshot.overlay.rightClickKeepsEdits'), 'info');
  }
}

function onDoubleClick(e: MouseEvent) {
  if (!selection.value || tool.value || !containsPoint(selection.value, framePoint(e))) return;
  void doExport('copy');
}

// ───────────────────────────── 标注 ─────────────────────────────

function clampToSelection(p: Point): Point {
  const s = selection.value!;
  return {
    x: Math.min(Math.max(p.x, s.x), s.x + s.width),
    y: Math.min(Math.max(p.y, s.y), s.y + s.height),
  };
}

function beginAnnotation(p: Point) {
  const base = { id: nextAnnotationId(), color: color.value, size: frameSize.value };
  switch (tool.value) {
    case 'number':
      history.add({ ...base, kind: 'number', at: p, value: nextNumberValue(history.items) });
      historyVersion.value++;
      requestRender();
      return;
    case 'text':
      textEditor.value = { at: p, value: '', size: frameSize.value, color: color.value };
      void nextTick(() => textEl.value?.focus());
      return;
    case 'rect':
      draft.value = { ...base, kind: 'rect', rect: { x: p.x, y: p.y, width: 0, height: 0 } };
      break;
    case 'mosaic':
      draft.value = {
        ...base,
        kind: 'mosaic',
        color: '#000000',
        rect: { x: p.x, y: p.y, width: 0, height: 0 },
        block: mosaicBlockSize(frameSize.value),
      };
      break;
    case 'arrow':
      draft.value = { ...base, kind: 'arrow', from: p, to: p };
      break;
    default:
      return;
  }
  dragging.value = { kind: 'draw', start: p };
}

function updateDraft(a: Annotation, start: Point, p: Point): Annotation {
  switch (a.kind) {
    case 'rect':
    case 'mosaic':
      return { ...a, rect: rectFromPoints(start, p) };
    case 'arrow':
      return { ...a, to: p };
    default:
      return a;
  }
}

function commitText() {
  const editor = textEditor.value;
  if (!editor) return;
  textEditor.value = null;
  if (editor.value.trim()) {
    history.add({ id: nextAnnotationId(), kind: 'text', at: editor.at, text: editor.value, color: editor.color, size: editor.size });
    historyVersion.value++;
  }
  requestRender();
  rootEl.value?.focus();
}

function onTextKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    textEditor.value = null;
    rootEl.value?.focus();
  } else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    commitText();
  }
}

function toggleTool(next: AnnotationTool) {
  if (textEditor.value) commitText();
  tool.value = tool.value === next ? null : next;
  void measureToolbar();
}

function undo() {
  if (history.undo()) {
    historyVersion.value++;
    requestRender();
  }
}

function redo() {
  if (history.redo()) {
    historyVersion.value++;
    requestRender();
  }
}

async function measureToolbar() {
  await nextTick();
  if (toolbarEl.value) {
    toolbarSize.width = toolbarEl.value.offsetWidth;
    toolbarSize.height = toolbarEl.value.offsetHeight;
  }
}

// ───────────────────────────── 导出与取消 ─────────────────────────────

/** 渲染选区大小的透明标注层 PNG；没有标注时返回 null，后端直接裁剪原图。 */
async function renderOverlayPng(sel: Rect): Promise<Uint8Array | null> {
  if (!frame || history.items.length === 0) return null;
  const canvas = document.createElement('canvas');
  canvas.width = sel.width;
  canvas.height = sel.height;
  const ctx = canvas.getContext('2d')!;
  ctx.translate(-sel.x, -sel.y);
  renderAnnotations(ctx, history.items, frame, mosaicCache);
  const blob = await new Promise<Blob>((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error(t('screenshot.overlay.encodeFailed')))), 'image/png')
  );
  return new Uint8Array(await blob.arrayBuffer());
}

async function doExport(action: ExportAction) {
  const sel = selection.value;
  if (!sel || !session.value || exporting.value) return;
  if (textEditor.value) commitText();
  exporting.value = true;
  try {
    const png = await renderOverlayPng(sel);
    await exportSelection(session.value.session_id, sel, action, png);
    // 成功后后端销毁本窗口；以下状态仅在窗口尚未销毁时生效。
  } catch (err: any) {
    exporting.value = false;
    showToast(err?.message || String(err), 'error');
  }
}

async function cancelCapture() {
  try {
    await screenshotApi.cancel(session.value?.session_id);
  } catch (err: any) {
    // 会话已结束时直接销毁自身，避免留下无法关闭的遮罩。
    (window as any).__TAURI__?.window?.getCurrentWindow?.()?.destroy?.();
    console.warn('取消截图失败:', err);
  }
}

async function copyProbeColor(asRgb: boolean) {
  if (!probeColor.value) return;
  const text = asRgb ? probeRgb.value : probeHex.value;
  try {
    await screenshotApi.copyColor(text);
    showToast(t('screenshot.overlay.colorCopied', { color: text }), 'info');
  } catch (err: any) {
    showToast(err?.message || String(err), 'error');
  }
}

function showToast(text: string, kind: 'info' | 'error') {
  toast.value = { text, kind };
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = null), kind === 'error' ? 4000 : 1600);
}

// ───────────────────────────── 键盘 ─────────────────────────────

function moveProbe(dx: number, dy: number) {
  probe.value = {
    x: Math.min(Math.max(0, probe.value.x + dx), bounds.value.width - 1),
    y: Math.min(Math.max(0, probe.value.y + dy), bounds.value.height - 1),
  };
  requestRender();
}

function onKeyDown(e: KeyboardEvent) {
  if (!ready.value && e.key !== 'Escape') return;
  if (textEditor.value) return;
  const mod = e.metaKey || e.ctrlKey;
  const key = e.key;
  if (key === 'Escape') {
    e.preventDefault();
    void cancelCapture();
    return;
  }
  if (exporting.value) return;
  if (e.code === 'Space') {
    e.preventDefault();
    spaceHeld.value = true;
    return;
  }
  if (mod && (key === 'z' || key === 'Z')) {
    e.preventDefault();
    if (e.shiftKey) redo();
    else undo();
    return;
  }
  if (mod && (key === 'y' || key === 'Y')) {
    e.preventDefault();
    redo();
    return;
  }
  if (mod && (key === 's' || key === 'S')) {
    e.preventDefault();
    void doExport('save');
    return;
  }
  if ((mod && (key === 'c' || key === 'C')) || key === 'Enter') {
    e.preventDefault();
    void doExport('copy');
    return;
  }
  if (key === 'F3') {
    e.preventDefault();
    void doExport('pin');
    return;
  }
  if (key === 'Tab' && !selection.value) {
    e.preventDefault();
    levelSteps = Math.min(8, Math.max(0, levelSteps + (e.shiftKey ? -1 : 1)));
    scheduleResolve(probe.value);
    return;
  }
  if (key.startsWith('Arrow')) {
    e.preventDefault();
    if (selection.value) {
      selection.value = nudgeRect(selection.value, key as ArrowKey, e.shiftKey, bounds.value);
      void measureToolbar();
      requestRender();
    } else {
      moveProbe(key === 'ArrowLeft' ? -1 : key === 'ArrowRight' ? 1 : 0, key === 'ArrowUp' ? -1 : key === 'ArrowDown' ? 1 : 0);
    }
    return;
  }
  if (mod) return;
  const lower = key.toLowerCase();
  if (lower === 'w' || lower === 'a' || lower === 's' || lower === 'd') {
    moveProbe(lower === 'a' ? -1 : lower === 'd' ? 1 : 0, lower === 'w' ? -1 : lower === 's' ? 1 : 0);
    return;
  }
  if (lower === 'c') {
    void copyProbeColor(e.shiftKey);
    return;
  }
  const index = Number(key) - 1;
  if (selection.value && Number.isInteger(index) && index >= 0 && index < ANNOTATION_TOOLS.length) {
    toggleTool(ANNOTATION_TOOLS[index]);
  }
}

function onKeyUp(e: KeyboardEvent) {
  if (e.code === 'Space') spaceHeld.value = false;
}

function onResize() {
  viewport.width = window.innerWidth;
  viewport.height = window.innerHeight;
  requestRender();
}

// ───────────────────────────── 生命周期 ─────────────────────────────

function nextFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

onMounted(async () => {
  document.body.classList.add('capture-overlay-body');
  window.addEventListener('keydown', onKeyDown);
  window.addEventListener('keyup', onKeyUp);
  window.addEventListener('resize', onResize);
  try {
    const info = await screenshotApi.getSession();
    session.value = info;
    // 后端已校验默认值；调色板和线宽按钮是快捷选项，不限制用户保存的自定义值。
    color.value = info.settings.default_color;
    strokeWidth.value = info.settings.default_stroke_width;
    const buffer = await fetchFrozenFrame(info.session_id);
    if (buffer.byteLength !== info.frame_byte_length) {
      throw new Error(t('screenshot.overlay.frameSizeMismatch'));
    }
    const rgba = new Uint8ClampedArray(buffer);
    frame = { rgba, width: info.frame_width, height: info.frame_height };
    for (const canvas of [baseCanvas.value!, uiCanvas.value!]) {
      canvas.width = info.frame_width;
      canvas.height = info.frame_height;
    }
    baseCanvas.value!.getContext('2d')!.putImageData(new ImageData(rgba, info.frame_width, info.frame_height), 0, 0);
    onResize();
    probe.value = { x: Math.floor(info.frame_width / 2), y: Math.floor(info.frame_height / 2) };
    ready.value = true;
    renderUi();
    // 等两帧确保冻结帧已合成到屏幕，再通知宿主显示遮罩。
    await nextFrame();
    await nextFrame();
    await screenshotApi.overlayReady(info.session_id);
    rootEl.value?.focus();
  } catch (err: any) {
    loadError.value = err?.message || String(err);
    try {
      if (session.value) await screenshotApi.overlayReady(session.value.session_id);
      else await screenshotApi.cancel();
    } catch {
      await cancelCapture();
    }
  }
});

onUnmounted(() => {
  window.removeEventListener('keydown', onKeyDown);
  window.removeEventListener('keyup', onKeyUp);
  window.removeEventListener('resize', onResize);
  if (toastTimer) clearTimeout(toastTimer);
  mosaicCache.clear();
  frame = null;
});
</script>

<style scoped>
.capture-root {
  position: fixed;
  inset: 0;
  overflow: hidden;
  background: #000;
  outline: none;
  user-select: none;
  -webkit-user-select: none;
}
.cursor-cross { cursor: crosshair; }
.cursor-move { cursor: move; }
.cursor-grab { cursor: grab; }
.cursor-grabbing { cursor: grabbing; }

.layer {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

/* DOM 交互层统一高于两层画布 */
.capture-hint,
.region-label,
.size-badge,
.handle,
.text-editor,
.capture-toolbar,
.magnifier,
.capture-toast,
.capture-error {
  position: absolute;
  z-index: 10;
}

.capture-hint {
  top: 16px;
  left: 50%;
  transform: translateX(-50%);
  padding: 6px 12px;
  border-radius: 8px;
  background: rgba(28, 28, 30, 0.82);
  color: #f5f5f7;
  font-size: 12px;
  pointer-events: none;
}

.region-label,
.size-badge {
  padding: 2px 8px;
  border-radius: 5px;
  background: rgba(10, 132, 255, 0.92);
  color: #fff;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  pointer-events: none;
}

.handle {
  width: 8px;
  height: 8px;
  background: #fff;
  border: 1.5px solid #0a84ff;
  border-radius: 2px;
  z-index: 12;
}
.handle-top_left, .handle-bottom_right { cursor: nwse-resize; }
.handle-top_right, .handle-bottom_left { cursor: nesw-resize; }
.handle-top, .handle-bottom { cursor: ns-resize; }
.handle-left, .handle-right { cursor: ew-resize; }

.text-editor {
  min-width: 120px;
  min-height: 1.4em;
  padding: 0;
  margin: 0;
  border: 1px dashed rgba(255, 255, 255, 0.85);
  background: rgba(0, 0, 0, 0.12);
  outline: none;
  resize: both;
  overflow: hidden;
  z-index: 14;
}

.capture-toolbar {
  z-index: 20;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 5px;
  border-radius: 10px;
  background: rgba(36, 36, 38, 0.94);
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.35);
  cursor: default;
}
.toolbar-row {
  display: flex;
  align-items: center;
  gap: 2px;
}
.sub-row {
  padding-top: 4px;
  border-top: 1px solid rgba(255, 255, 255, 0.1);
}
.tool-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: #e5e5ea;
  cursor: pointer;
}
.tool-btn:hover:not(:disabled) { background: rgba(255, 255, 255, 0.12); }
.tool-btn.active { background: rgba(10, 132, 255, 0.9); color: #fff; }
.tool-btn.primary { color: #30d158; }
.tool-btn:disabled { opacity: 0.35; cursor: default; }
.tool-btn.text-btn { width: auto; padding: 0 12px; background: rgba(255, 255, 255, 0.12); }
.sep {
  width: 1px;
  height: 18px;
  margin: 0 4px;
  background: rgba(255, 255, 255, 0.16);
}
.color-dot {
  width: 18px;
  height: 18px;
  margin: 0 2px;
  border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, 0.25);
  cursor: pointer;
}
.color-dot.active { border-color: #fff; box-shadow: 0 0 0 2px #0a84ff; }
.width-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: transparent;
  cursor: pointer;
}
.width-btn.active { background: rgba(255, 255, 255, 0.16); }
.width-dot { display: block; border-radius: 50%; background: #e5e5ea; }

.magnifier {
  width: 122px;
  pointer-events: none;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.5);
  background: rgba(28, 28, 30, 0.92);
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.4);
  z-index: 16;
}
.mag-canvas {
  display: block;
  width: 120px;
  height: 120px;
  image-rendering: pixelated;
}
.mag-info {
  padding: 5px 7px 6px;
  color: #f5f5f7;
  font-size: 11px;
  line-height: 1.45;
  font-variant-numeric: tabular-nums;
}
.mag-color { display: flex; align-items: center; gap: 5px; }
.swatch {
  width: 10px;
  height: 10px;
  border-radius: 2px;
  border: 1px solid rgba(255, 255, 255, 0.6);
}
.mag-hint { color: #a1a1a6; font-size: 10px; }

.capture-toast {
  bottom: 28px;
  left: 50%;
  transform: translateX(-50%);
  padding: 8px 14px;
  border-radius: 8px;
  background: rgba(28, 28, 30, 0.9);
  color: #f5f5f7;
  font-size: 12px;
  pointer-events: none;
  z-index: 30;
}
.capture-toast.error { background: rgba(255, 69, 58, 0.95); }

.capture-error {
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  gap: 10px;
  align-items: center;
  max-width: 520px;
  padding: 20px 24px;
  border-radius: 12px;
  background: rgba(28, 28, 30, 0.95);
  color: #f5f5f7;
  font-size: 13px;
  text-align: center;
  z-index: 40;
}
</style>

<style>
/* 遮罩窗口不透明黑底，避免冻结帧绘制前露出白色背景 */
body.capture-overlay-body {
  background: #000 !important;
  margin: 0;
  overflow: hidden;
}
</style>
