<template>
  <div class="pin-mirror-frame">
    <img
      v-if="snapshotUrl"
      :src="snapshotUrl"
      :alt="t('winpin.mirror.alt')"
      class="mirror-viewport"
      @click="activateOriginalWindow"
    />
    <div v-else class="loading-state">
      <span>{{ t('winpin.mirror.loading') }}</span>
    </div>

    <!-- 悬浮微型快捷栏 -->
    <div class="mirror-overlay-bar">
      <button class="glass-pill-btn" @click="activateOriginalWindow" :title="t('winpin.mirror.activateTooltip')">
        <span>{{ t('winpin.mirror.activateBtn') }}</span>
      </button>
      <button class="glass-pill-btn danger" @click="unpinWindow" :title="t('winpin.mirror.unpinTooltip')">
        <span>{{ t('winpin.mirror.unpinBtn') }}</span>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';

const handle = Number(new URLSearchParams(window.location.search).get('handle') || 0);
const snapshotUrl = ref('');
let timer: any = null;

async function fetchSnapshot() {
  if (!handle) return;
  try {
    const res = await invokeIpc<{ handle: number; data_url: string }>('winpin_get_snapshot', {
      handle,
    });
    if (res?.data_url) {
      snapshotUrl.value = res.data_url;
    }
  } catch (err) {
    // 捕获无录屏权限等异常
  }
}

async function activateOriginalWindow() {
  if (!handle) return;
  try {
    await invokeIpc('winpin_activate', { handle });
  } catch (err) {
    console.error('激活原窗口失败:', err);
  }
}

async function unpinWindow() {
  if (!handle) return;
  try {
    await invokeIpc('winpin_unpin', { handle });
    (window as any).__TAURI__?.window?.getCurrentWindow?.()?.destroy?.();
  } catch (err) {
    console.error('取消置顶失败:', err);
  }
}

onMounted(() => {
  fetchSnapshot();
  timer = setInterval(fetchSnapshot, 700);
});

onUnmounted(() => {
  clearInterval(timer);
});
</script>

<style scoped>
.pin-mirror-frame {
  position: fixed;
  inset: 0;
  background: #000;
  border: 1px solid rgba(255, 69, 58, 0.6);
  border-radius: 8px;
  overflow: hidden;
  box-sizing: border-box;
}

.mirror-viewport {
  width: 100%;
  height: 100%;
  object-fit: fill;
  display: block;
  cursor: pointer;
}

.loading-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #8e8e93;
  font-size: 12px;
}

.mirror-overlay-bar {
  position: absolute;
  top: 8px;
  right: 8px;
  display: flex;
  gap: 6px;
  z-index: 50;
}

.glass-pill-btn {
  background: rgba(30, 30, 32, 0.85);
  backdrop-filter: blur(12px);
  border: 1px solid rgba(255, 255, 255, 0.15);
  color: #ffffff;
  font-size: 11px;
  font-weight: 500;
  padding: 4px 9px;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.12s ease;
}

.glass-pill-btn:hover {
  background: #0a84ff;
  border-color: transparent;
}

.glass-pill-btn.danger:hover {
  background: #ff453a;
}
</style>
