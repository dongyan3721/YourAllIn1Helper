<template>
  <div class="screen-ink-canvas-root" :class="{ 'cursor-crosshair': currentTool !== 'eraser' && currentTool !== 'laser', 'cursor-eraser': currentTool === 'eraser', 'cursor-laser': currentTool === 'laser' }">
    <canvas
      ref="canvasRef"
      class="ink-canvas"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
    ></canvas>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import {
  type InkTool,
  type InkPoint,
  type InkStroke,
  InkHistoryManager,
  LaserPointerManager,
  renderStroke,
} from '../utils/ink';
import { invokeIpc } from '../api/ipc';

const canvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;

// 当前绘图设置
const currentTool = ref<InkTool>('pen');
const currentColor = ref<string>('#ef4444');
const currentWidth = ref<number>(4);

// 历史记录与激光笔管理器
const historyManager = new InkHistoryManager();
const laserManager = new LaserPointerManager();

// 当前活跃笔画
let isDrawing = false;
let currentPoints: InkPoint[] = [];
let animFrameId: number | null = null;

// 与独立工具栏双向通信的跨窗口广播通道
let broadcastChannel: BroadcastChannel | null = null;

onMounted(() => {
  initCanvas();
  window.addEventListener('resize', onResize);

  // 初始化同源广播信道（保证透明画布与独立工具栏零网络毫秒级同步）
  broadcastChannel = new BroadcastChannel('screen-ink-channel');
  broadcastChannel.onmessage = (event) => {
    handleToolbarMessage(event.data);
  };

  // 通知工具栏画布已就绪
  notifyToolbar();
});

onUnmounted(() => {
  window.removeEventListener('resize', onResize);
  if (animFrameId) {
    cancelAnimationFrame(animFrameId);
  }
  if (broadcastChannel) {
    broadcastChannel.close();
  }
});

function initCanvas() {
  const canvas = canvasRef.value;
  if (!canvas) return;

  const dpr = window.devicePixelRatio || 1;
  const w = window.innerWidth;
  const h = window.innerHeight;

  canvas.width = w * dpr;
  canvas.height = h * dpr;
  canvas.style.width = `${w}px`;
  canvas.style.height = `${h}px`;

  ctx = canvas.getContext('2d');
  if (ctx) {
    ctx.scale(dpr, dpr);
  }
  redrawAll();
}

function onResize() {
  initCanvas();
}

/** 处理工具栏发送的控制指令 */
async function handleToolbarMessage(data: any) {
  if (!data || !data.type) return;

  switch (data.type) {
    case 'set_tool':
      currentTool.value = data.tool;
      break;
    case 'set_color':
      currentColor.value = data.color;
      break;
    case 'set_width':
      currentWidth.value = data.width;
      break;
    case 'undo':
      historyManager.undo();
      redrawAll();
      notifyToolbar();
      break;
    case 'redo':
      historyManager.redo();
      redrawAll();
      notifyToolbar();
      break;
    case 'clear':
      historyManager.clear();
      laserManager.clear();
      redrawAll();
      notifyToolbar();
      break;
    case 'request_snapshot':
      await handleExportSnapshot(data.copyToClipboard);
      break;
  }
}

function notifyToolbar() {
  if (!broadcastChannel) return;
  broadcastChannel.postMessage({
    type: 'state_sync',
    canUndo: historyManager.canUndo(),
    canRedo: historyManager.canRedo(),
    tool: currentTool.value,
    color: currentColor.value,
    width: currentWidth.value,
  });
}

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return; // 仅响应主按键
  isDrawing = true;
  const target = e.currentTarget as HTMLElement;
  target.setPointerCapture(e.pointerId);

  const pt: InkPoint = {
    x: e.clientX,
    y: e.clientY,
    pressure: e.pressure > 0 ? e.pressure : 0.5,
    timestamp: Date.now(),
  };

  currentPoints = [pt];

  if (currentTool.value === 'laser') {
    laserManager.addPoint(pt.x, pt.y);
    startLaserLoop();
  }
}

function onPointerMove(e: PointerEvent) {
  if (!isDrawing) return;

  const pt: InkPoint = {
    x: e.clientX,
    y: e.clientY,
    pressure: e.pressure > 0 ? e.pressure : 0.5,
    timestamp: Date.now(),
  };

  currentPoints.push(pt);

  if (currentTool.value === 'laser') {
    laserManager.addPoint(pt.x, pt.y);
    startLaserLoop();
    return;
  }

  // 几何图形与实时笔画绘制
  if (['line', 'rect', 'arrow'].includes(currentTool.value)) {
    redrawAll();
    if (ctx) {
      renderStroke(ctx, {
        id: 'preview',
        tool: currentTool.value,
        color: currentColor.value,
        width: currentWidth.value,
        points: [currentPoints[0], pt],
      });
    }
  } else {
    // 自由笔画（pen, highlighter, eraser）
    if (ctx) {
      const segLen = currentPoints.length;
      if (segLen >= 2) {
        renderStroke(ctx, {
          id: 'incremental',
          tool: currentTool.value,
          color: currentColor.value,
          width: currentWidth.value,
          points: currentPoints.slice(segLen - 3 > 0 ? segLen - 3 : 0),
        });
      }
    }
  }
}

function onPointerUp(e: PointerEvent) {
  if (!isDrawing) return;
  isDrawing = false;
  const target = e.currentTarget as HTMLElement;
  try {
    target.releasePointerCapture(e.pointerId);
  } catch (_) {}

  if (currentTool.value === 'laser') {
    currentPoints = [];
    return;
  }

  if (currentPoints.length > 0) {
    const newStroke: InkStroke = {
      id: `stroke_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
      tool: currentTool.value,
      color: currentColor.value,
      width: currentWidth.value,
      points: [...currentPoints],
    };
    historyManager.push(newStroke);
    redrawAll();
    notifyToolbar();
  }
  currentPoints = [];
}

/** 全量重绘画布所有已有笔画 */
function redrawAll() {
  const canvas = canvasRef.value;
  if (!canvas || !ctx) return;

  // 清空全屏像素
  ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);

  // 顺序绘制所有历史笔画
  for (const stroke of historyManager.strokes) {
    renderStroke(ctx, stroke);
  }
}

/** 激光笔消隐渐变循环渲染 */
function startLaserLoop() {
  if (animFrameId !== null) return;

  const loop = () => {
    if (!laserManager.hasPoints()) {
      animFrameId = null;
      redrawAll();
      return;
    }

    redrawAll();
    if (ctx) {
      laserManager.render(ctx, currentColor.value, Date.now());
    }
    animFrameId = requestAnimationFrame(loop);
  };

  animFrameId = requestAnimationFrame(loop);
}

/** 导出当前墨迹透明 PNG 并调用微内核真实合成落盘/复制到剪贴板 */
async function handleExportSnapshot(copyToClipboard: boolean = true) {
  const canvas = canvasRef.value;
  if (!canvas) return;

  // 临时导出仅包含有效墨迹的透明 PNG（不含激光笔临时亮点）
  const offscreen = document.createElement('canvas');
  const dpr = window.devicePixelRatio || 1;
  const w = window.innerWidth;
  const h = window.innerHeight;
  offscreen.width = w * dpr;
  offscreen.height = h * dpr;
  const offCtx = offscreen.getContext('2d');
  if (offCtx) {
    offCtx.scale(dpr, dpr);
    for (const stroke of historyManager.strokes) {
      renderStroke(offCtx, stroke);
    }
  }

  const inkPngDataUrl = offscreen.toDataURL('image/png');

  try {
    const res = await invokeIpc('ink_save_snapshot', {
      ink_png_base64: inkPngDataUrl,
      copy_to_clipboard: copyToClipboard,
    });

    if (broadcastChannel) {
      broadcastChannel.postMessage({
        type: 'snapshot_result',
        success: true,
        savedPath: res.saved_path,
        copiedToClipboard: res.copied_to_clipboard,
      });
    }
  } catch (err: any) {
    if (broadcastChannel) {
      broadcastChannel.postMessage({
        type: 'snapshot_result',
        success: false,
        error: err.message || '截图合成保存失败',
      });
    }
  }
}
</script>

<style scoped>
.screen-ink-canvas-root {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  margin: 0;
  padding: 0;
  background: transparent !important;
  overflow: hidden;
  user-select: none;
  touch-action: none;
}

.ink-canvas {
  display: block;
  width: 100vw;
  height: 100vh;
  background: transparent !important;
}

.cursor-crosshair {
  cursor: crosshair;
}

.cursor-eraser {
  cursor: cell;
}

.cursor-laser {
  cursor: none;
}
</style>
