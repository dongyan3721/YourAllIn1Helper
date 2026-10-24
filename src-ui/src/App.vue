<template>
  <div class="app-root" :class="{ 'native-window': isNativeWindowMode }">
    <!-- 剪贴板快速粘贴弹窗视图 -->
    <ClipboardPopup v-if="nativeWindowType === 'clipboard-popup'" />

    <!-- 桌面置顶富文本便笺独立窗口视图 -->
    <StickyNote v-else-if="nativeWindowType === 'sticky-note'" :note-id="nativeStickyId" />

    <!-- 窗口置顶全屏准星拾取层视图 -->
    <WindowPinPicker v-else-if="nativeWindowType === 'window-pin-picker'" />

    <!-- macOS 原生置顶镜像画中画视图 -->
    <PinMirror v-else-if="nativeWindowType === 'pin-mirror'" />

    <!-- 默认主控台视图 -->
    <Dashboard v-else />
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import Dashboard from './components/Dashboard.vue';
import ClipboardPopup from './components/ClipboardPopup.vue';
import StickyNote from './components/StickyNote.vue';
import WindowPinPicker from './components/WindowPinPicker.vue';
import PinMirror from './components/PinMirror.vue';

// 解析原生 Tauri 多窗口参数
const urlParams = new URLSearchParams(window.location.search);
const isNativeWindowMode = ref(urlParams.get('mode') === 'native');
const rawWindow = urlParams.get('window') || '';
const nativeWindowType = ref(
  rawWindow.startsWith('sticky-note')
    ? 'sticky-note'
    : rawWindow.startsWith('pin-mirror')
    ? 'pin-mirror'
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
