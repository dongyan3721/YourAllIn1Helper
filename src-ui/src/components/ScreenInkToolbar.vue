<template>
  <div class="screen-ink-toolbar-root" data-tauri-drag-region>
    <div class="capsule-toolbar" :class="{ 'passthrough-active': isPassthrough }">
      <!-- 拖拽手柄 -->
      <div class="drag-grip" data-tauri-drag-region :title="t('screen_ink.dragTooltip')">
        <span class="grip-line"></span>
        <span class="grip-line"></span>
      </div>

      <!-- 工具选择组 (7种工具) -->
      <div class="tool-group">
        <button
          v-for="tool in TOOLS"
          :key="tool.id"
          class="tb-btn"
          :class="{ active: currentTool === tool.id && !isPassthrough }"
          :title="t(tool.tooltipKey)"
          @click="selectTool(tool.id)"
        >
          <Icons :name="tool.icon" size="13" />
        </button>
      </div>

      <div class="divider"></div>

      <!-- 调色板选择器 -->
      <div class="color-picker-wrapper">
        <button
          class="color-trigger-btn"
          :style="{ backgroundColor: currentColor }"
          :title="t('screen_ink.colorTooltip')"
          @click="showColorMenu = !showColorMenu"
        ></button>
        <!-- 弹层展示在 130px 高度窗口内，不超出不裁切 -->
        <div v-if="showColorMenu" class="color-popover">
          <button
            v-for="c in COLOR_PALETTE"
            :key="c"
            class="color-swatch"
            :style="{ backgroundColor: c }"
            :class="{ active: currentColor === c }"
            @click="selectColor(c)"
          ></button>
        </div>
      </div>

      <!-- 线宽调节器 -->
      <div class="width-picker-wrapper">
        <button
          v-for="w in WIDTH_PRESETS"
          :key="w.val"
          class="width-dot-btn"
          :class="{ active: currentWidth === w.val }"
          :title="t(w.labelKey)"
          @click="selectWidth(w.val)"
        >
          <span
            class="dot-preview"
            :style="{ width: `${w.val * 1.4}px`, height: `${w.val * 1.4}px`, backgroundColor: currentColor }"
          ></span>
        </button>
      </div>

      <div class="divider"></div>

      <!-- 撤销 / 重做 / 清屏 -->
      <div class="action-group">
        <button
          class="tb-btn"
          :disabled="!canUndo || isPassthrough"
          :title="t('screen_ink.undo')"
          @click="triggerUndo"
        >
          <Icons name="undo" size="13" />
        </button>
        <button
          class="tb-btn"
          :disabled="!canRedo || isPassthrough"
          :title="t('screen_ink.redo')"
          @click="triggerRedo"
        >
          <Icons name="redo" size="13" />
        </button>
        <button
          class="tb-btn danger"
          :disabled="isPassthrough"
          :title="t('screen_ink.clear')"
          @click="triggerClear"
        >
          <Icons name="trash" size="13" />
        </button>
      </div>

      <div class="divider"></div>

      <!-- 桌面穿透开关 (核心关键：让画布穿透，工具栏自身保持可点) -->
      <button
        class="tb-btn passthrough-btn"
        :class="{ active: isPassthrough }"
        :title="isPassthrough ? t('screen_ink.passthroughActiveTooltip') : t('screen_ink.passthroughInactiveTooltip')"
        @click="togglePassthrough"
      >
        <Icons name="mouse-pointer" size="13" />
        <span v-if="isPassthrough" class="status-indicator"></span>
      </button>

      <!-- 合成快照保存 / 复制 -->
      <button
        class="tb-btn primary"
        :title="t('screen_ink.saveTooltip')"
        :disabled="isSaving"
        @click="triggerSave"
      >
        <Icons name="camera" size="13" />
      </button>

      <!-- 退出全屏注释 -->
      <button
        class="tb-btn close-btn"
        :title="t('screen_ink.exitTooltip')"
        @click="exitSession"
      >
        <Icons name="close" size="13" />
      </button>
    </div>

    <!-- 保存轻提示 Toast (位于 130px 窗口内部，保证完全不被边缘裁剪) -->
    <transition name="toast-fade">
      <div v-if="toastMessage" class="toast-pill">
        <span>{{ toastMessage }}</span>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import { type InkTool } from '../utils/ink';
import Icons from './Icons.vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';

interface ToolItem {
  id: InkTool;
  icon: string;
  tooltipKey: string;
}

const TOOLS: ToolItem[] = [
  { id: 'pen', icon: 'brush', tooltipKey: 'screen_ink.tools.pen' },
  { id: 'highlighter', icon: 'highlighter', tooltipKey: 'screen_ink.tools.highlighter' },
  { id: 'laser', icon: 'laser', tooltipKey: 'screen_ink.tools.laser' },
  { id: 'line', icon: 'line', tooltipKey: 'screen_ink.tools.line' },
  { id: 'rect', icon: 'rect', tooltipKey: 'screen_ink.tools.rect' },
  { id: 'arrow', icon: 'arrow', tooltipKey: 'screen_ink.tools.arrow' },
  { id: 'eraser', icon: 'eraser', tooltipKey: 'screen_ink.tools.eraser' },
];

const COLOR_PALETTE = [
  '#ef4444', // 红
  '#f97316', // 橙
  '#eab308', // 黄
  '#22c55e', // 绿
  '#3b82f6', // 蓝
  '#a855f7', // 紫
  '#ffffff', // 白
  '#000000', // 黑
];

const WIDTH_PRESETS = [
  { val: 2, labelKey: 'screen_ink.widths.thin' },
  { val: 4, labelKey: 'screen_ink.widths.medium' },
  { val: 8, labelKey: 'screen_ink.widths.thick' },
];

const currentTool = ref<InkTool>('pen');
const currentColor = ref<string>('#ef4444');
const currentWidth = ref<number>(4);
const showColorMenu = ref<boolean>(false);

const canUndo = ref<boolean>(false);
const canRedo = ref<boolean>(false);
const isPassthrough = ref<boolean>(false);
const isSaving = ref<boolean>(false);
const toastMessage = ref<string | null>(null);

let broadcastChannel: BroadcastChannel | null = null;
let toastTimeout: number | null = null;
let unlistenTauriMode: (() => void) | null = null;

onMounted(async () => {
  // 1. 初始化同源广播信道
  broadcastChannel = new BroadcastChannel('screen-ink-channel');
  broadcastChannel.onmessage = (event) => {
    handleCanvasMessage(event.data);
  };

  // 2. 关键：启动时主动请求画布状态（解决工具栏先于画布或后于画布加载的竞态时序问题）
  broadcastChannel.postMessage({ type: 'request_state_sync' });

  // 3. 从后端查询权威运行态状态（包含当前穿透模式与配置）
  try {
    const status = await invokeIpc<{
      is_passthrough: boolean;
      active_tool?: string;
      color?: string;
      stroke_width?: number;
    }>('ink_get_status');
    isPassthrough.value = status.is_passthrough;
    if (status.active_tool) currentTool.value = status.active_tool as InkTool;
    if (status.color) currentColor.value = status.color;
    if (status.stroke_width) currentWidth.value = status.stroke_width;
  } catch (_) {}

  // 4. 监听原生 Tauri 事件桥（快捷键/主控台切换穿透时立即同步更新工具栏）
  const tauriListen = (window as any).__TAURI__?.event?.listen;
  if (typeof tauriListen === 'function') {
    try {
      unlistenTauriMode = await tauriListen('screen-ink://mode-changed', (event: any) => {
        const payload = event.payload;
        if (payload && typeof payload.is_passthrough === 'boolean') {
          isPassthrough.value = payload.is_passthrough;
        }
      });
    } catch (_) {}
  }
});

onUnmounted(() => {
  if (broadcastChannel) {
    broadcastChannel.close();
  }
  if (unlistenTauriMode) {
    unlistenTauriMode();
    unlistenTauriMode = null;
  }
  if (toastTimeout) {
    clearTimeout(toastTimeout);
  }
});

function handleCanvasMessage(data: any) {
  if (!data || !data.type) return;

  if (data.type === 'state_sync') {
    if (typeof data.canUndo === 'boolean') canUndo.value = data.canUndo;
    if (typeof data.canRedo === 'boolean') canRedo.value = data.canRedo;
    if (data.tool) currentTool.value = data.tool;
    if (data.color) currentColor.value = data.color;
    if (data.width) currentWidth.value = data.width;
  } else if (data.type === 'snapshot_result') {
    isSaving.value = false;
    if (data.success) {
      if (data.copiedToClipboard) {
        showToast(t('screen_ink.toastSaveSuccess'));
      } else if (data.clipboardError) {
        showToast(`${t('screen_ink.toastSaveFileOnly')} (${data.clipboardError})`);
      } else {
        showToast(t('screen_ink.toastSaveSuccess'));
      }
    } else {
      showToast(data.error || t('screen_ink.toastSaveFailed'));
    }
  }
}

function selectTool(tool: InkTool) {
  currentTool.value = tool;
  if (isPassthrough.value) {
    // 切换工具自动切回绘图模式
    togglePassthrough();
  }
  broadcastChannel?.postMessage({ type: 'set_tool', tool });
}

function selectColor(color: string) {
  currentColor.value = color;
  showColorMenu.value = false;
  broadcastChannel?.postMessage({ type: 'set_color', color });
}

function selectWidth(width: number) {
  currentWidth.value = width;
  broadcastChannel?.postMessage({ type: 'set_width', width });
}

function triggerUndo() {
  broadcastChannel?.postMessage({ type: 'undo' });
}

function triggerRedo() {
  broadcastChannel?.postMessage({ type: 'redo' });
}

function triggerClear() {
  broadcastChannel?.postMessage({ type: 'clear' });
}

async function togglePassthrough() {
  const target = !isPassthrough.value;
  try {
    const res = await invokeIpc('ink_set_passthrough', { passthrough: target });
    isPassthrough.value = res.is_passthrough;
    showToast(isPassthrough.value ? t('screen_ink.toastPassthroughOn') : t('screen_ink.toastPassthroughOff'));
  } catch (err: any) {
    showToast(err.message || '切换桌面穿透失败');
  }
}

function triggerSave() {
  isSaving.value = true;
  showToast(t('screen_ink.toastSaving'));
  broadcastChannel?.postMessage({ type: 'request_snapshot', copyToClipboard: true });
}

async function exitSession() {
  try {
    await invokeIpc('ink_exit_session');
  } catch (_) {}
}

function showToast(msg: string) {
  toastMessage.value = msg;
  if (toastTimeout) {
    clearTimeout(toastTimeout);
  }
  toastTimeout = window.setTimeout(() => {
    toastMessage.value = null;
  }, 2600);
}
</script>

<style scoped>
.screen-ink-toolbar-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  padding-top: 4px;
  background: transparent !important;
  user-select: none;
  pointer-events: auto;
  overflow: visible;
  position: relative;
}

.capsule-toolbar {
  display: flex;
  align-items: center;
  gap: 3px;
  height: 42px;
  padding: 0 10px;
  border-radius: 21px;
  background: rgba(255, 255, 255, 0.88);
  backdrop-filter: blur(24px) saturate(180%);
  -webkit-backdrop-filter: blur(24px) saturate(180%);
  border: 1px solid rgba(0, 0, 0, 0.12);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18), 0 2px 6px rgba(0, 0, 0, 0.08);
  transition: all 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  flex-shrink: 0;
}

:global(.dark) .capsule-toolbar {
  background: rgba(36, 40, 48, 0.90);
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
}

.capsule-toolbar.passthrough-active {
  border-color: #3b82f6;
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.35), 0 8px 24px rgba(0, 0, 0, 0.2);
}

.drag-grip {
  display: flex;
  flex-direction: column;
  gap: 3px;
  cursor: grab;
  padding: 6px 3px;
}

.grip-line {
  display: block;
  width: 12px;
  height: 2px;
  border-radius: 1px;
  background: rgba(0, 0, 0, 0.25);
}

:global(.dark) .grip-line {
  background: rgba(255, 255, 255, 0.3);
}

.tool-group,
.action-group {
  display: flex;
  align-items: center;
  gap: 1px;
}

.tb-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 27px;
  height: 27px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: #333333;
  cursor: pointer;
  transition: all 0.15s ease;
  position: relative;
  flex-shrink: 0;
}

:global(.dark) .tb-btn {
  color: #e5e5e5;
}

.tb-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.07);
}

:global(.dark) .tb-btn:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.12);
}

.tb-btn.active {
  background: #3b82f6;
  color: #ffffff;
}

.tb-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.tb-btn.danger:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
}

.tb-btn.close-btn:hover {
  background: rgba(239, 68, 68, 0.2);
  color: #ef4444;
}

.tb-btn.passthrough-btn {
  position: relative;
}

.status-indicator {
  position: absolute;
  top: 3px;
  right: 3px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #10b981;
  box-shadow: 0 0 6px #10b981;
}

.divider {
  width: 1px;
  height: 18px;
  background: rgba(0, 0, 0, 0.1);
  margin: 0 2px;
  flex-shrink: 0;
}

:global(.dark) .divider {
  background: rgba(255, 255, 255, 0.15);
}

/* 颜色选择器 */
.color-picker-wrapper {
  position: relative;
  display: flex;
  align-items: center;
}

.color-trigger-btn {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid #ffffff;
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.2);
  cursor: pointer;
  transition: transform 0.15s ease;
  flex-shrink: 0;
}

.color-trigger-btn:hover {
  transform: scale(1.1);
}

.color-popover {
  position: absolute;
  top: 34px;
  left: 50%;
  transform: translateX(-50%);
  display: grid;
  grid-template-columns: repeat(4, 18px);
  gap: 5px;
  padding: 6px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(0, 0, 0, 0.12);
  box-shadow: 0 8px 18px rgba(0, 0, 0, 0.22);
  z-index: 1000;
}

:global(.dark) .color-popover {
  background: rgba(30, 32, 38, 0.96);
  border-color: rgba(255, 255, 255, 0.18);
}

.color-swatch {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
  transition: transform 0.15s ease;
}

.color-swatch.active {
  border-color: #3b82f6;
  transform: scale(1.15);
}

/* 线宽调节器 */
.width-picker-wrapper {
  display: flex;
  align-items: center;
  gap: 2px;
}

.width-dot-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 4px;
  border: none;
  background: transparent;
  cursor: pointer;
  flex-shrink: 0;
}

.width-dot-btn.active {
  background: rgba(0, 0, 0, 0.08);
}

:global(.dark) .width-dot-btn.active {
  background: rgba(255, 255, 255, 0.12);
}

.dot-preview {
  border-radius: 50%;
  display: block;
}

/* 提示 Toast */
.toast-pill {
  margin-top: 6px;
  padding: 4px 14px;
  border-radius: 12px;
  background: rgba(0, 0, 0, 0.82);
  color: #ffffff;
  font-size: 11px;
  backdrop-filter: blur(8px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
  pointer-events: none;
  z-index: 999;
}

.toast-fade-enter-active,
.toast-fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.toast-fade-enter-from,
.toast-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
