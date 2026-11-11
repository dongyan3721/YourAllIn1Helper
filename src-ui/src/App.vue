<template>
  <div class="app-root" :class="{ 'native-window': isNativeWindowMode }">
    <!-- 剪贴板快速粘贴弹窗视图 -->
    <SaveToast v-if="nativeWindowType.startsWith('save-toast-')" />
    <ClipboardPopup v-else-if="nativeWindowType === 'clipboard-popup'" />

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

    <FeaturePanel v-else-if="nativeWindowType.startsWith('feature-')" />

    <!-- 主窗口只包含功能与偏好设置 -->
    <Dashboard v-else />
    <div v-if="toastMessage" class="app-toast" :class="{ error: toastMessage.error }" role="status">{{ toastMessage.text }}</div>
  </div>
</template>

<script setup lang="ts">
import { ref, defineAsyncComponent } from 'vue';
import Dashboard from './components/Dashboard.vue';
import { toastMessage } from './utils/toast';
const SaveToast = defineAsyncComponent(() => import('./components/SaveToast.vue'));
const FeaturePanel = defineAsyncComponent(() => import('./components/FeaturePanel.vue'));
const ClipboardPopup = defineAsyncComponent(() => import('./components/ClipboardPopup.vue'));
const StickyNote = defineAsyncComponent(() => import('./components/StickyNote.vue'));
const WindowPinPicker = defineAsyncComponent(() => import('./components/WindowPinPicker.vue'));
const PinMirror = defineAsyncComponent(() => import('./components/PinMirror.vue'));
const PinBadge = defineAsyncComponent(() => import('./components/PinBadge.vue'));
const ScreenInkCanvas = defineAsyncComponent(() => import('./components/ScreenInkCanvas.vue'));
const ScreenInkToolbar = defineAsyncComponent(() => import('./components/ScreenInkToolbar.vue'));
const CaptureOverlay = defineAsyncComponent(() => import('./components/CaptureOverlay.vue'));
const PinImage = defineAsyncComponent(() => import('./components/PinImage.vue'));
const ScreenshotControl = defineAsyncComponent(() => import('./components/ScreenshotControl.vue'));
const ScreenshotPreview = defineAsyncComponent(() => import('./components/ScreenshotPreview.vue'));
const TranslatePopover = defineAsyncComponent(() => import('./components/TranslatePopover.vue'));
const TranslateTrigger = defineAsyncComponent(() => import('./components/TranslateTrigger.vue'));
import { initTheme } from './utils/theme';
import { initI18n } from './utils/i18n';

// 全局初始化 macOS 主题（白蓝色调、主题色与明暗跟随）与多语言国际化监听器
initTheme();
initI18n();

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
.app-toast { position: fixed; z-index: 10000; bottom: 24px; left: 50%; transform: translateX(-50%); max-width: min(640px, calc(100vw - 32px)); padding: 12px 18px; background: var(--macos-bg-card-hover); color: var(--macos-text-primary); border: 1px solid var(--macos-border-subtle); box-shadow: var(--macos-shadow-popup); border-radius: 10px; overflow-wrap: anywhere; }
.app-toast.error { border-color: var(--macos-danger); }
</style>
