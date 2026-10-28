<template>
  <div class="preview-root">
    <!-- 顶部状态栏与操作按钮 -->
    <header class="preview-header" @mousedown="startWindowDrag">
      <div class="title-info">
        <span class="type-tag">{{ isScroll ? t('screenshot.scroll.previewTitle') : t('screenshot.gif.previewTitle') }}</span>
        <span v-if="dimensions" class="dim-badge">{{ dimensions.w }} × {{ dimensions.h }}</span>
        <span v-if="stopReasonText" class="reason-badge">{{ stopReasonText }}</span>
      </div>

      <div class="header-actions">
        <template v-if="isScroll">
          <button class="action-btn" :disabled="loading || exporting" @click="doExport('pin')">
            <Icons name="pin" :size="15" />
            <span>{{ t('screenshot.overlay.pin') }}</span>
          </button>
          <button class="action-btn" :disabled="loading || exporting" @click="doExport('save')">
            <Icons name="download" :size="15" />
            <span>{{ t('screenshot.overlay.save') }}</span>
          </button>
          <button class="action-btn primary" :disabled="loading || exporting" @click="doExport('copy')">
            <Icons name="check" :size="15" />
            <span>{{ t('screenshot.overlay.copy') }}</span>
          </button>
        </template>

        <template v-else-if="gifStatus">
          <span class="gif-metric">{{ formatGifDuration(gifStatus.duration_ms) }}</span>
          <span class="gif-metric">{{ gifStatus.frame_count }} {{ t('screenshot.gif.frames') }}</span>
          <span class="gif-metric">{{ formatFileSize(gifStatus.file_size_bytes) }}</span>
        </template>

        <button class="action-btn close-btn" @click="close">
          <Icons name="close" :size="15" />
        </button>
      </div>
    </header>

    <!-- 中间预览内容区 -->
    <main class="preview-content">
      <div v-if="loading" class="preview-loading">
        <div class="spinner"></div>
        <span>{{ t('screenshot.scroll.loadingPreview') }}</span>
      </div>

      <!-- 长截图预览：支持纵向无缝长图滚动 -->
      <div v-else-if="isScroll && scrollCanvasUrl" class="scroll-preview-wrapper">
        <img :src="scrollCanvasUrl" class="scroll-img" alt="Stitched Screenshot" />
      </div>

      <!-- GIF 录制预览 -->
      <div v-else-if="!isScroll && gifStatus?.saved_path" class="gif-preview-wrapper">
        <div class="gif-card">
          <div class="gif-icon">🎬</div>
          <div class="gif-title">{{ t('screenshot.gif.savedSuccess') }}</div>
          <div class="gif-path" :title="gifStatus.saved_path">{{ gifStatus.saved_path }}</div>
        </div>
      </div>

      <div v-if="toast" class="preview-toast" :class="toast.kind">{{ toast.text }}</div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { startWindowDrag } from '../utils/windowDrag';
import { ref, computed, onMounted, onUnmounted } from 'vue';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';
import {
  screenshotApi,
  fetchScrollImage,
  type ExportAction,
  type GifStatus,
} from '../api/screenshot';
import { formatGifDuration, formatFileSize } from '../utils/screenshot/gif';

const urlParams = new URLSearchParams(window.location.search);
const isScroll = computed(() => urlParams.get('type') !== 'gif');

const loading = ref(true);
const exporting = ref(false);
const scrollCanvasUrl = ref<string | null>(null);
const dimensions = ref<{ w: number; h: number } | null>(null);
const stopReasonText = ref<string | null>(null);
const gifStatus = ref<GifStatus | null>(null);
const toast = ref<{ text: string; kind: 'info' | 'error' } | null>(null);

onMounted(async () => {
  if (isScroll.value) {
    try {
      const infoRes = await screenshotApi.scrollInfo();
      if (infoRes) {
        dimensions.value = { w: infoRes.width, h: infoRes.current_height };
        if (infoRes.stop_reason?.reason === 'low_confidence') {
          stopReasonText.value = t('screenshot.scroll.lowConfidenceTip');
        } else if (infoRes.stop_reason?.reason === 'max_length_reached') {
          stopReasonText.value = t('screenshot.scroll.maxLengthTip');
        }
      }

      // 获取二进制 RGBA 原始字节并在 Canvas 转换为图片展示
      const buffer = await fetchScrollImage();
      const rawBytes = new Uint8ClampedArray(buffer);
      if (dimensions.value && rawBytes.length > 0) {
        const canvas = document.createElement('canvas');
        canvas.width = dimensions.value.w;
        canvas.height = dimensions.value.h;
        const ctx = canvas.getContext('2d');
        if (ctx) {
          const imgData = new ImageData(rawBytes, dimensions.value.w, dimensions.value.h);
          ctx.putImageData(imgData, 0, 0);
          scrollCanvasUrl.value = canvas.toDataURL('image/png');
        }
      }
    } catch (e: any) {
      showToast(e?.message || '加载长截图预览失败', 'error');
    } finally {
      loading.value = false;
    }
  } else {
    try {
      const res = await screenshotApi.gifStatus();
      if (res) {
        gifStatus.value = res;
        dimensions.value = { w: res.width, h: res.height };
      }
    } catch (e: any) {
      showToast(e?.message || '读取 GIF 录制状态失败', 'error');
    } finally {
      loading.value = false;
    }
  }
});

onUnmounted(() => {
  scrollCanvasUrl.value = null;
});

function showToast(text: string, kind: 'info' | 'error' = 'info') {
  toast.value = { text, kind };
  setTimeout(() => {
    toast.value = null;
  }, 3000);
}

async function doExport(action: ExportAction) {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const outcome = await screenshotApi.scrollExport(action);
    if (action === 'copy') {
      showToast(t('screenshot.scroll.copySuccess'), 'info');
    } else if (action === 'save') {
      showToast(t('screenshot.scroll.saveSuccess') + (outcome.saved_path ? `: ${outcome.saved_path}` : ''), 'info');
    } else if (action === 'pin') {
      showToast(t('screenshot.scroll.pinSuccess'), 'info');
    }
  } catch (e: any) {
    showToast(e?.message || '导出长截图失败', 'error');
    exporting.value = false;
  }
}

async function close() {
  try {
    if (isScroll.value) {
      await screenshotApi.scrollCancel();
    } else {
      await screenshotApi.gifCancel();
    }
  } catch (_) {}
}
</script>

<style scoped>
.preview-root {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  background: #1c1c1e;
  color: #fff;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  user-select: none;
  overflow: hidden;
}

.preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 52px;
  padding: 0 16px;
  background: #2c2c2e;
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
}

.title-info {
  display: flex;
  align-items: center;
  gap: 10px;
}

.type-tag {
  font-weight: 600;
  font-size: 14px;
  letter-spacing: 0.3px;
}

.dim-badge {
  background: rgba(255, 255, 255, 0.1);
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.7);
  font-variant-numeric: tabular-nums;
}

.reason-badge {
  background: rgba(255, 149, 0, 0.2);
  color: #ff9500;
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 500;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.gif-metric {
  font-size: 13px;
  color: rgba(255, 255, 255, 0.75);
  background: rgba(255, 255, 255, 0.08);
  padding: 4px 10px;
  border-radius: 8px;
}

.action-btn {
  display: flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  padding: 0 12px;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.12);
  color: #fff;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.action-btn:hover {
  background: rgba(255, 255, 255, 0.22);
}

.action-btn.primary {
  background: #007aff;
}

.action-btn.primary:hover {
  background: #0062cc;
}

.close-btn {
  padding: 0 8px;
  background: transparent;
  color: rgba(255, 255, 255, 0.6);
}

.close-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.preview-content {
  flex: 1;
  min-height: 0;
  position: relative;
  overflow-y: auto;
  overflow-x: hidden;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding: 20px;
  background: #141416;
}

.preview-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  height: 100%;
  color: rgba(255, 255, 255, 0.6);
}

.spinner {
  width: 28px;
  height: 28px;
  border: 3px solid rgba(255, 255, 255, 0.15);
  border-top-color: #007aff;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.scroll-preview-wrapper {
  max-width: 100%;
  flex: 0 0 auto;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  border-radius: 4px;
}

.scroll-img {
  display: block;
  max-width: 100%;
  height: auto;
}

.gif-preview-wrapper {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
}

.gif-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  background: #242426;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 12px;
  padding: 28px 36px;
  max-width: 500px;
  text-align: center;
}

.gif-icon {
  font-size: 40px;
}

.gif-title {
  font-size: 16px;
  font-weight: 600;
  color: #fff;
}

.gif-path {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.6);
  word-break: break-all;
  background: rgba(0, 0, 0, 0.3);
  padding: 8px 12px;
  border-radius: 6px;
  font-family: ui-monospace, Menlo, Consolas, monospace;
}

.preview-toast {
  position: absolute;
  bottom: 24px;
  left: 50%;
  transform: translateX(-50%);
  padding: 8px 18px;
  border-radius: 20px;
  background: rgba(0, 0, 0, 0.85);
  backdrop-filter: blur(12px);
  color: #fff;
  font-size: 13px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.preview-toast.error {
  background: rgba(255, 59, 48, 0.9);
}
</style>
