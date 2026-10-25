<template>
  <div class="control-root" tabindex="0" data-tauri-drag-region @keydown="onControlKey">
    <!-- 模式 1：长截图进行中 -->
    <template v-if="mode === 'scroll'">
      <div class="status-indicator scroll-indicator">
        <span class="pulsing-dot green"></span>
        <span class="indicator-text">{{ t('screenshot.scroll.scrolling') }}</span>
      </div>

      <div class="metric-badge">
        <span class="metric-label">{{ t('screenshot.scroll.height') }}:</span>
        <span class="metric-value">{{ formatPixelHeight(scrollHeight) }}</span>
      </div>

      <div class="actions">
        <button
          class="ctrl-btn auto-btn"
          :class="{ active: autoScrollActive }"
          :title="t('screenshot.scroll.autoScrollTitle')"
          @click="toggleAutoScroll"
        >
          {{ autoScrollActive ? t('screenshot.scroll.stopAuto') : t('screenshot.scroll.autoScroll') }}
        </button>
        <button class="ctrl-btn stop-btn" :title="t('screenshot.scroll.finishTitle')" @click="stopScroll">
          <Icons name="check" :size="14" />
          <span>{{ t('screenshot.scroll.finish') }}</span>
        </button>
        <button class="ctrl-btn cancel-btn" :title="t('screenshot.scroll.cancelTitle')" @click="cancelScroll">
          <Icons name="close" :size="14" />
        </button>
      </div>
    </template>

    <!-- 模式 2：GIF 录制进行中 -->
    <template v-else-if="mode === 'gif'">
      <div class="status-indicator gif-indicator">
        <span class="pulsing-dot red"></span>
        <span class="indicator-text">{{ formatGifDuration(gifDuration) }}</span>
      </div>

      <div class="metric-badge">
        <span class="metric-value">{{ gifFrameCount }} {{ t('screenshot.gif.frames') }}</span>
        <span class="sep">|</span>
        <span class="metric-value">{{ formatFileSize(gifFileSize) }}</span>
      </div>

      <div class="actions">
        <button class="ctrl-btn stop-btn red-btn" :title="t('screenshot.gif.stopTitle')" @click="stopGif">
          <span class="stop-square"></span>
          <span>{{ t('screenshot.gif.stop') }}</span>
        </button>
        <button class="ctrl-btn cancel-btn" :title="t('screenshot.gif.cancelTitle')" @click="cancelGif">
          <Icons name="close" :size="14" />
        </button>
      </div>
    </template>

    <div v-if="errorToast" class="error-tip">{{ errorToast }}</div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';
import { screenshotApi } from '../api/screenshot';
import { formatPixelHeight } from '../utils/screenshot/scroll';
import { formatGifDuration, formatFileSize } from '../utils/screenshot/gif';

const urlParams = new URLSearchParams(window.location.search);
const mode = ref<'scroll' | 'gif'>(urlParams.get('mode') === 'gif' ? 'gif' : 'scroll');

// 长截图状态
const scrollHeight = ref(0);
const autoScrollActive = ref(false);

// GIF 状态
const gifDuration = ref(0);
const gifFrameCount = ref(0);
const gifFileSize = ref(0);

const errorToast = ref<string | null>(null);
let unlistenFn: (() => void) | null = null;
let pollTimer: ReturnType<typeof setInterval> | null = null;

function onControlKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.preventDefault();
    if (mode.value === 'gif') void cancelGif();
    else void cancelScroll();
  } else if (e.key === 'Enter') {
    e.preventDefault();
    if (mode.value === 'gif') void stopGif();
    else void stopScroll();
  }
}

onMounted(async () => {
  // 订阅 Tauri 事件
  try {
    const tauriEvent = (window as any).__TAURI__?.event;
    if (tauriEvent?.listen) {
      if (mode.value === 'scroll') {
        const unlisten = await tauriEvent.listen('screenshot://scroll-frame-stitched', (event: any) => {
          if (event.payload?.current_height) {
            scrollHeight.value = event.payload.current_height;
          }
        });
        unlistenFn = unlisten;
      } else {
        const unlisten = await tauriEvent.listen('screenshot://gif-recording-stats', (event: any) => {
          if (event.payload) {
            gifDuration.value = event.payload.duration_ms ?? 0;
            gifFrameCount.value = event.payload.frame_count ?? 0;
            gifFileSize.value = event.payload.file_size_bytes ?? 0;
          }
        });
        unlistenFn = unlisten;
      }
    }
  } catch (e) {
    // 降级为轮询
    startPolling();
  }

  // 兜底定时轮询
  startPolling();
});

onUnmounted(() => {
  if (unlistenFn) unlistenFn();
  if (pollTimer) clearInterval(pollTimer);
});

function startPolling() {
  pollTimer = setInterval(async () => {
    try {
      if (mode.value === 'scroll') {
        const res = await screenshotApi.scrollStatus();
        if (res) {
          scrollHeight.value = res.current_height;
          autoScrollActive.value = res.auto_scroll_enabled;
          if (res.auto_scroll_error) {
            showError(res.auto_scroll_error);
          }
        }
      } else {
        const res = await screenshotApi.gifStatus();
        if (res) {
          gifDuration.value = res.duration_ms;
          gifFrameCount.value = res.frame_count;
          gifFileSize.value = res.file_size_bytes;
        }
      }
    } catch (_) {}
  }, 300);
}

function showError(msg: string) {
  errorToast.value = msg;
  setTimeout(() => {
    errorToast.value = null;
  }, 4000);
}

async function toggleAutoScroll() {
  const next = !autoScrollActive.value;
  autoScrollActive.value = next;
  try {
    await screenshotApi.scrollSetAuto(next);
  } catch (e: any) {
    autoScrollActive.value = false;
    showError(e?.message || '自动滚动失败');
  }
}

async function stopScroll() {
  try {
    await screenshotApi.scrollStop();
  } catch (e: any) {
    showError(e?.message || '停止长截图失败');
  }
}

async function cancelScroll() {
  try {
    await screenshotApi.scrollCancel();
  } catch (_) {}
}

async function stopGif() {
  try {
    await screenshotApi.gifStop();
  } catch (e: any) {
    showError(e?.message || '停止 GIF 录制失败');
  }
}

async function cancelGif() {
  try {
    await screenshotApi.gifCancel();
  } catch (_) {}
}
</script>

<style scoped>
.control-root {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  padding: 0 12px;
  background: rgba(28, 30, 36, 0.92);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 26px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
  color: #fff;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  font-size: 13px;
  user-select: none;
  cursor: default;
}

.status-indicator {
  display: flex;
  align-items: center;
  gap: 8px;
}

.pulsing-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.pulsing-dot.green {
  background: #34c759;
  box-shadow: 0 0 8px #34c759;
  animation: pulse-green 1.4s infinite ease-in-out;
}

.pulsing-dot.red {
  background: #ff3b30;
  box-shadow: 0 0 8px #ff3b30;
  animation: pulse-red 1.2s infinite ease-in-out;
}

@keyframes pulse-green {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.4; transform: scale(0.85); }
}

@keyframes pulse-red {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.35; transform: scale(0.85); }
}

.indicator-text {
  font-weight: 600;
  letter-spacing: 0.3px;
}

.metric-badge {
  display: flex;
  align-items: center;
  gap: 6px;
  background: rgba(255, 255, 255, 0.08);
  padding: 4px 10px;
  border-radius: 12px;
  font-variant-numeric: tabular-nums;
  font-size: 12px;
}

.metric-label {
  color: rgba(255, 255, 255, 0.6);
}

.metric-value {
  color: #e5e5ea;
  font-weight: 500;
}

.sep {
  color: rgba(255, 255, 255, 0.2);
}

.actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ctrl-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.12);
  color: #fff;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.ctrl-btn:hover {
  background: rgba(255, 255, 255, 0.22);
}

.auto-btn.active {
  background: #007aff;
  color: #fff;
}

.stop-btn {
  background: #007aff;
}

.stop-btn:hover {
  background: #0062cc;
}

.red-btn {
  background: #ff3b30;
}

.red-btn:hover {
  background: #d70015;
}

.stop-square {
  width: 9px;
  height: 9px;
  background: #fff;
  border-radius: 2px;
}

.cancel-btn {
  padding: 0 8px;
  background: transparent;
  color: rgba(255, 255, 255, 0.7);
}

.cancel-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: #fff;
}

.error-tip {
  position: absolute;
  top: calc(100% + 6px);
  left: 10px;
  right: 10px;
  background: #ff3b30;
  color: #fff;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 11px;
  text-align: center;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}
</style>
