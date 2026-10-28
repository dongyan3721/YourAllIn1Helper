<template>
  <div class="app-root" :class="{ 'native-window': isNativeWindowMode }">
    <!-- 剪贴板快速粘贴弹窗视图 -->
    <ClipboardPopup v-if="nativeWindowType === 'clipboard-popup'" />

    <!-- 桌面置顶富文本便笺独立窗口视图 -->
    <StickyNote v-else-if="nativeWindowType === 'sticky-note'" :note-id="nativeStickyId" />

    <!-- 窗口置顶全屏准星拾取层视图 -->
    <WindowPinPicker v-else-if="nativeWindowType === 'window-pin-picker'" />

    <!-- macOS 原生置顶轻量图钉角标视图 -->
    <PinBadge v-else-if="nativeWindowType === 'pin-badge'" />

    <!-- macOS 原生置顶镜像画中画视图 -->
    <PinMirror v-else-if="nativeWindowType === 'pin-mirror'" />

    <!-- 桌面屏幕注释透明画布视图 -->
    <ScreenInkCanvas v-else-if="nativeWindowType === 'screen-ink-canvas'" />

    <!-- 桌面屏幕注释独立工具栏视图 -->
    <ScreenInkToolbar v-else-if="nativeWindowType === 'screen-ink-toolbar'" />

    <!-- 增强截图：冻结帧遮罩与独立贴图 -->
    <CaptureOverlay v-else-if="nativeWindowType === 'capture-overlay'" />
    <PinImage v-else-if="nativeWindowType === 'pin-image'" />
    <ScreenshotControl v-else-if="nativeWindowType === 'screenshot-control'" />
    <ScreenshotPreview v-else-if="nativeWindowType === 'screenshot-preview'" />
    <!-- 全局划词翻译浮动气泡独立窗口视图 -->
    <TranslateTrigger v-else-if="nativeWindowType === 'translate-trigger'" />
    <TranslatePopover v-else-if="nativeWindowType === 'translate-popover'" />

    <!-- 默认主控台视图 -->
    <Dashboard v-else />
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue';
import Dashboard from './components/Dashboard.vue';
import ClipboardPopup from './components/ClipboardPopup.vue';
import StickyNote from './components/StickyNote.vue';
import WindowPinPicker from './components/WindowPinPicker.vue';
import PinMirror from './components/PinMirror.vue';
import PinBadge from './components/PinBadge.vue';
import ScreenInkCanvas from './components/ScreenInkCanvas.vue';
import ScreenInkToolbar from './components/ScreenInkToolbar.vue';
import CaptureOverlay from './components/CaptureOverlay.vue';
import PinImage from './components/PinImage.vue';
import ScreenshotControl from './components/ScreenshotControl.vue';
import ScreenshotPreview from './components/ScreenshotPreview.vue';
import TranslatePopover from './components/TranslatePopover.vue';
import TranslateTrigger from './components/TranslateTrigger.vue';
import { initTheme } from './utils/theme';
import { initI18n } from './utils/i18n';

// 全局初始化 macOS 主题（白蓝色调、主题色与明暗跟随）与多语言国际化监听器
initTheme();
initI18n();
onMounted(() => {
  initTheme();
  initI18n();
});

// 解析原生 Tauri 多窗口参数
const urlParams = new URLSearchParams(window.location.search);
const isNativeWindowMode = ref(urlParams.get('mode') === 'native');
const rawWindow = urlParams.get('window') || '';
const nativeWindowType = ref(
  rawWindow.startsWith('sticky-note')
    ? 'sticky-note'
    : rawWindow.startsWith('pin-badge')
    ? 'pin-badge'
    : rawWindow.startsWith('pin-mirror')
    ? 'pin-mirror'
    : rawWindow.startsWith('pin-img-')
    ? 'pin-image'
    : rawWindow
);
const nativeStickyId = ref(
  urlParams.get('id') || (rawWindow.startsWith('sticky-note-') ? rawWindow.replace('sticky-note-', '') : '')
);

if (isNativeWindowMode.value) {
  document.body.classList.add('native-window-mode');
}
</script>

<style scoped>
.app-root {
  width: 100vw;
  height: 100vh;
  overflow: hidden;
}
</style>
