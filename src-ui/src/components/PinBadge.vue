<template>
  <div
    class="pin-badge-container"
    :title="t('winpin.badge.tooltip')"
    @click="unpinWindow"
  >
    <div class="pin-badge-disc">
      <Icons name="pin-fill" size="13" class="pin-icon" />
      <Icons name="close" size="11" class="unpin-hover-icon" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { invokeIpc } from '../api/ipc';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';

// 解析当前图钉角标对应的目标窗口 handle
const handle = Number(new URLSearchParams(window.location.search).get('handle') || 0);

/** 点击图钉角标：解除目标外部窗口置顶并销毁角标窗口 */
async function unpinWindow() {
  if (!handle) return;
  try {
    // 1. 调用 winpin_unpin 恢复目标物理窗口为正常层级并触发后端销毁
    await invokeIpc('winpin_unpin', { handle });
    // 2. 双重保障：调用内核通用窗口销毁命令立即销毁角标
    await invokeIpc('kernel_destroy_window', { label: `pin-badge-${handle}` });
  } catch (err) {
    console.error('解除窗口置顶失败:', err);
  }
}
</script>

<style scoped>
/* 整个窗口保持完全透明且无边框 */
.pin-badge-container {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  user-select: none;
  background: transparent;
}

/* 经典 macOS 原生高质感悬浮图钉微圆盘 */
.pin-badge-disc {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: var(--macos-bg-card, rgba(255, 255, 255, 0.9));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid var(--macos-border-subtle, rgba(0, 0, 0, 0.12));
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.18), 0 0 0 1px rgba(var(--macos-accent-rgb, 0, 122, 255), 0.2);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.15s ease, background 0.15s ease, box-shadow 0.15s ease;
  position: relative;
}

.pin-icon {
  color: var(--macos-accent, #007aff);
  transition: opacity 0.15s ease;
}

.unpin-hover-icon {
  position: absolute;
  color: var(--macos-danger, #ff3b30);
  opacity: 0;
  transition: opacity 0.15s ease;
}

.pin-badge-container:hover .pin-badge-disc {
  transform: scale(1.1);
  background: var(--macos-bg-card-hover, rgba(255, 255, 255, 0.98));
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.24);
}

.pin-badge-container:hover .pin-icon {
  opacity: 0;
}

.pin-badge-container:hover .unpin-hover-icon {
  opacity: 1;
}
</style>
