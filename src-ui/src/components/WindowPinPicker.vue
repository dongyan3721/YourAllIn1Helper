<template>
  <div class="pin-picker-overlay" @contextmenu.prevent="cancelPicker" @keydown.esc="cancelPicker" tabindex="0">
    <!-- 顶部超轻量无干扰提示胶囊 -->
    <div class="picker-hud-pill">
      <Icons name="crosshair" size="13" />
      <span>{{ t('winpin.picker.hudHint') }}</span>
    </div>

    <!-- 真实外部窗口感应热区层（默认完全透明无全屏蒙层，悬浮时高亮当前候选窗口） -->
    <div
      v-for="win in candidates"
      :key="win.handle"
      class="candidate-box"
      :style="getBoxStyle(win)"
      @click="pickWindow(win)"
    >
      <div class="candidate-tag">
        <Icons name="pin" size="11" />
        <span class="app-name">{{ win.process_name }}</span>
        <span class="win-title">{{ win.title }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invokeIpc, type ExternalWindowInfo } from '../api/ipc';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';

/** 候选外部窗口列表 */
const candidates = ref<ExternalWindowInfo[]>([]);

/** 屏幕缩放因子计算 */
const scaleX = ref(1);
const scaleY = ref(1);

/** 样式计算映射 */
function getBoxStyle(w: ExternalWindowInfo) {
  const left = Math.max(0, w.x * scaleX.value);
  const top = Math.max(0, w.y * scaleY.value);
  const width = Math.max(40, w.width * scaleX.value);
  const height = Math.max(40, w.height * scaleY.value);

  return {
    left: `${left}px`,
    top: `${top}px`,
    width: `${width}px`,
    height: `${height}px`,
  };
}

/** 点击选择窗口并置顶 */
async function pickWindow(w: ExternalWindowInfo) {
  const centerX = w.x + Math.floor(w.width / 2);
  const centerY = w.y + Math.floor(w.height / 2);
  try {
    await invokeIpc('winpin_pick_at_cursor', { x: centerX, y: centerY });
  } catch (err) {
    console.error('置顶外部窗口失败:', err);
  }
}

/** 取消准星拾取模式 */
async function cancelPicker() {
  try {
    await invokeIpc('kernel_destroy_window', { label: 'window-pin-picker' });
  } catch (err) {
    console.error('取消拾取器失败:', err);
  }
}

onMounted(async () => {
  scaleX.value = window.innerWidth / (window.screen.width || window.innerWidth);
  scaleY.value = window.innerHeight / (window.screen.height || window.innerHeight);

  try {
    const res = await invokeIpc<{ candidates: ExternalWindowInfo[] }>('winpin_list_candidates');
    candidates.value = res.candidates || [];
  } catch (err) {
    console.error('获取外部窗口列表失败:', err);
  }
});
</script>

<style scoped>
/* 准星拾取全屏层：完全透明，不添加任何半透明蒙层或高斯模糊，纯靠 cursor: crosshair 十字准星 */
.pin-picker-overlay {
  position: fixed;
  inset: 0;
  background: transparent;
  cursor: crosshair;
  user-select: none;
  outline: none;
}

/* 顶部极简毛玻璃辅助提示条 */
.picker-hud-pill {
  position: absolute;
  top: 18px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--macos-bg-card, rgba(28, 28, 30, 0.85));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid var(--macos-border-subtle, rgba(255, 255, 255, 0.15));
  border-radius: 999px;
  padding: 6px 16px;
  color: var(--macos-text-primary, #f5f5f7);
  font-size: 12px;
  font-weight: 500;
  display: flex;
  align-items: center;
  gap: 8px;
  box-shadow: var(--macos-shadow-popup, 0 8px 24px rgba(0, 0, 0, 0.2));
  z-index: 1000;
  pointer-events: none;
}

/* 候选窗口热区：平时完全透明，只有准星悬浮时显示微弱边框 */
.candidate-box {
  position: absolute;
  border: 1.5px dashed transparent;
  background: transparent;
  border-radius: 6px;
  cursor: crosshair;
  transition: border-color 0.12s ease, background 0.12s ease;
  overflow: hidden;
  box-sizing: border-box;
}

.candidate-box:hover {
  border-color: var(--macos-accent, #007aff);
  border-style: solid;
  background: var(--macos-accent-subtle, rgba(0, 122, 255, 0.08));
  z-index: 100;
}

/* 准星悬浮时左上角浮现的极简窗口标识 */
.candidate-tag {
  opacity: 0;
  background: var(--macos-accent, #007aff);
  color: #ffffff;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-bottom-right-radius: 4px;
  transition: opacity 0.12s ease;
}

.candidate-box:hover .candidate-tag {
  opacity: 1;
}

.app-name {
  font-weight: 600;
}

.win-title {
  opacity: 0.9;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
