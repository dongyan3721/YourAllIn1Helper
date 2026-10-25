<template>
  <div
    class="pin-root"
    :class="{ collapsed }"
    @wheel.prevent="onWheel"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="pressed = null"
    @dblclick="toggleCollapse"
    @mouseenter="hover = true"
    @mouseleave="hover = false"
    @contextmenu.prevent="toolbarPinned = !toolbarPinned"
  >
    <img v-if="src" :src="src" class="pin-image" :style="{ opacity }" draggable="false" alt="" />
    <div v-if="error" class="pin-error" :title="error">{{ error }}</div>

    <div
      v-if="(hover || toolbarPinned) && !collapsed && src"
      class="pin-toolbar"
      @pointerdown.stop
      @dblclick.stop
      @wheel.stop
    >
      <span class="pin-zoom">{{ formatPercent(zoom) }}</span>
      <button class="pin-btn" :title="t('screenshot.pin.copy') + ' (⌘/Ctrl+C)'" @click="copyPin">
        <Icons name="copy" :size="13" />
      </button>
      <button class="pin-btn" :title="t('screenshot.pin.save') + ' (⌘/Ctrl+S)'" @click="savePin">
        <Icons name="download" :size="13" />
      </button>
      <button class="pin-btn danger" :title="t('screenshot.pin.close') + ' (Esc)'" @click="closePin">
        <Icons name="close" :size="13" />
      </button>
    </div>

    <div v-if="badge" class="pin-badge">{{ badge }}</div>
  </div>
</template>

<script setup lang="ts">
/**
 * 贴图窗口（pin-img-{id}）：拖动、滚轮缩放 10%–500%、Shift+滚轮透明度、双击折叠、关闭销毁。
 *
 * PNG 经二进制 IPC 获取后转为 Blob URL 显示；窗口尺寸随缩放比例调整（需要 screenshot-pins 能力），
 * 关闭时由后端销毁窗口并释放 PNG，卸载时同步回收 Blob URL。
 */
import { onMounted, onUnmounted, ref } from 'vue';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';
import { fetchPinImage, screenshotApi } from '../api/screenshot';
import { formatPercent, pinWindowSize, stepOpacity, stepZoom, wheelDelta } from '../utils/screenshot/pinView';

const label = new URLSearchParams(window.location.search).get('window') || '';
const pinId = label.startsWith('pin-img-') ? label.slice('pin-img-'.length) : '';

const src = ref('');
const error = ref('');
const zoom = ref(1);
const opacity = ref(1);
const collapsed = ref(false);
const hover = ref(false);
const toolbarPinned = ref(false);
const badge = ref('');
const pressed = ref<{ x: number; y: number } | null>(null);
let baseWidth = 0;
let baseHeight = 0;
let badgeTimer: ReturnType<typeof setTimeout> | undefined;

function tauriWindow(): any {
  return (window as any).__TAURI__?.window?.getCurrentWindow?.();
}

function flash(text: string) {
  badge.value = text;
  if (badgeTimer) clearTimeout(badgeTimer);
  badgeTimer = setTimeout(() => (badge.value = ''), 1400);
}

/** 按缩放与折叠状态调整窗口逻辑尺寸；失败时如实提示（例如缺少窗口尺寸权限）。 */
async function applyWindowSize() {
  const win = tauriWindow();
  const tauri = (window as any).__TAURI__;
  const LogicalSize = tauri?.dpi?.LogicalSize ?? tauri?.window?.LogicalSize;
  if (!win || !LogicalSize) return;
  const size = pinWindowSize(baseWidth, baseHeight, zoom.value, collapsed.value);
  try {
    await win.setSize(new LogicalSize(size.width, size.height));
  } catch (err: any) {
    flash(t('screenshot.pin.resizeFailed', { error: err?.message || String(err) }));
  }
}

function onWheel(e: WheelEvent) {
  if (collapsed.value) return;
  const delta = wheelDelta(e);
  if (e.shiftKey || e.altKey) {
    opacity.value = stepOpacity(opacity.value, delta);
    flash(t('screenshot.pin.opacityBadge', { value: formatPercent(opacity.value) }));
  } else {
    const next = stepZoom(zoom.value, delta);
    if (next === zoom.value) return;
    zoom.value = next;
    flash(formatPercent(next));
    void applyWindowSize();
  }
}

function onPointerDown(e: PointerEvent) {
  if (e.button === 0) pressed.value = { x: e.screenX, y: e.screenY };
}

/** 按下后移动超过 3px 才交给系统拖动，保留单击与双击语义。 */
function onPointerMove(e: PointerEvent) {
  const start = pressed.value;
  if (!start || (e.buttons & 1) === 0) return;
  if (Math.abs(e.screenX - start.x) < 3 && Math.abs(e.screenY - start.y) < 3) return;
  pressed.value = null;
  tauriWindow()
    ?.startDragging?.()
    ?.catch?.((err: any) => flash(t('screenshot.pin.dragFailed', { error: err?.message || String(err) })));
}

function toggleCollapse() {
  collapsed.value = !collapsed.value;
  void applyWindowSize();
}

async function copyPin() {
  try {
    await screenshotApi.copyPin(pinId);
    flash(t('screenshot.pin.copied'));
  } catch (err: any) {
    flash(err?.message || String(err));
  }
}

async function savePin() {
  try {
    const res = await screenshotApi.savePin(pinId);
    flash(t('screenshot.pin.saved', { path: res.saved_path }));
  } catch (err: any) {
    flash(err?.message || String(err));
  }
}

async function closePin() {
  try {
    await screenshotApi.closePin(pinId);
  } catch (err) {
    // 模块已停用或记录已释放：直接销毁本窗口，不留下无法关闭的贴图。
    console.warn('关闭贴图失败，直接销毁窗口:', err);
    tauriWindow()?.destroy?.();
  }
}

function onKeyDown(e: KeyboardEvent) {
  const mod = e.metaKey || e.ctrlKey;
  if (e.key === 'Escape') {
    void closePin();
  } else if (mod && e.key.toLowerCase() === 'c') {
    e.preventDefault();
    void copyPin();
  } else if (mod && e.key.toLowerCase() === 's') {
    e.preventDefault();
    void savePin();
  } else if (!mod && (e.key === '+' || e.key === '=' || e.key === '-' || e.key === '0') && !collapsed.value) {
    zoom.value = e.key === '0' ? 1 : stepZoom(zoom.value, e.key === '-' ? 1 : -1);
    flash(formatPercent(zoom.value));
    void applyWindowSize();
  }
}

onMounted(async () => {
  document.body.classList.add('pin-image-body');
  window.addEventListener('keydown', onKeyDown);
  try {
    if (!pinId) throw new Error(t('screenshot.pin.invalidId'));
    const info = await screenshotApi.pinInfo(pinId);
    baseWidth = info.logical_width;
    baseHeight = info.logical_height;
    opacity.value = info.opacity;
    const png = await fetchPinImage(pinId);
    src.value = URL.createObjectURL(new Blob([png], { type: 'image/png' }));
  } catch (err: any) {
    error.value = err?.message || String(err);
  }
});

onUnmounted(() => {
  window.removeEventListener('keydown', onKeyDown);
  if (badgeTimer) clearTimeout(badgeTimer);
  if (src.value) URL.revokeObjectURL(src.value);
});
</script>

<style scoped>
.pin-root {
  position: fixed;
  inset: 0;
  overflow: hidden;
  cursor: grab;
  user-select: none;
  -webkit-user-select: none;
  box-shadow: inset 0 0 0 1px rgba(10, 132, 255, 0.55);
  background: transparent;
}
.pin-root.collapsed {
  border-radius: 8px;
  box-shadow: inset 0 0 0 2px rgba(10, 132, 255, 0.9);
}
.pin-image {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: fill;
  pointer-events: none;
}
.collapsed .pin-image {
  object-fit: cover;
  border-radius: 8px;
}
.pin-toolbar {
  position: absolute;
  top: 4px;
  right: 4px;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px 3px;
  border-radius: 7px;
  background: rgba(28, 28, 30, 0.86);
  cursor: default;
}
.pin-zoom {
  padding: 0 4px;
  color: #f5f5f7;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.pin-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: #e5e5ea;
  cursor: pointer;
}
.pin-btn:hover { background: rgba(255, 255, 255, 0.14); }
.pin-btn.danger:hover { background: rgba(255, 69, 58, 0.85); color: #fff; }
.pin-badge,
.pin-error {
  position: absolute;
  left: 50%;
  bottom: 6px;
  transform: translateX(-50%);
  max-width: calc(100% - 12px);
  padding: 3px 8px;
  border-radius: 6px;
  background: rgba(28, 28, 30, 0.86);
  color: #f5f5f7;
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  pointer-events: none;
}
.pin-error { background: rgba(255, 69, 58, 0.9); }
</style>

<style>
/* 贴图窗口透明背景，只显示图片本身 */
body.pin-image-body {
  background: transparent !important;
  margin: 0;
  overflow: hidden;
}
</style>
