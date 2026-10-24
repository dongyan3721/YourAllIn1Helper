<template>
  <div class="pin-picker-overlay" @contextmenu.prevent="cancelPicker" @keydown.esc="cancelPicker" tabindex="0">
    <!-- 顶部极简胶囊提示 -->
    <div class="picker-hud-pill">
      <Icons name="crosshair" size="14" />
      <span>点击任意真实应用窗口将其置顶最前 · 按 ESC 或右键取消</span>
    </div>

    <!-- 真实外部窗口感应矩形层 -->
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
.pin-picker-overlay {
  position: fixed;
  inset: 0;
  background: rgba(2, 6, 23, 0.4);
  backdrop-filter: blur(4px);
  cursor: crosshair;
  user-select: none;
  outline: none;
}

.picker-hud-pill {
  position: absolute;
  top: 24px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(24, 24, 27, 0.92);
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 999px;
  padding: 8px 18px;
  color: #f5f5f7;
  font-size: 13px;
  font-weight: 500;
  display: flex;
  align-items: center;
  gap: 8px;
  box-shadow: 0 12px 30px rgba(0, 0, 0, 0.4);
  z-index: 1000;
  pointer-events: none;
}

.candidate-box {
  position: absolute;
  border: 2px solid rgba(225, 29, 72, 0.65);
  background: rgba(225, 29, 72, 0.08);
  border-radius: 6px;
  cursor: crosshair;
  transition: all 0.12s ease;
  overflow: hidden;
  box-sizing: border-box;
}

.candidate-box:hover {
  border-color: #ff453a;
  background: rgba(255, 69, 58, 0.18);
  z-index: 100;
}

.candidate-tag {
  background: rgba(225, 29, 72, 0.9);
  color: #ffffff;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-bottom-right-radius: 4px;
}

.app-name {
  font-weight: 600;
}

.win-title {
  opacity: 0.85;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
