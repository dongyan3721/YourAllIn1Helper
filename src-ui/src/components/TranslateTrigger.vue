<template>
  <button class="translate-trigger" :title="t('translator.translateSelection')" :aria-label="t('translator.translateSelection')" :disabled="opening" @click="open">译</button>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';
const opening = ref(false);
async function open() {
  if (opening.value) return;
  opening.value = true;
  try { await invokeIpc('translator_open_mouse_selection'); }
  catch { /* 过期、停用时宿主会销毁入口；不重试旧选区。 */ }
  finally { opening.value = false; }
}
</script>

<style scoped>
.translate-trigger {
  display: block; width: 20px; height: 20px; padding: 0; border: 0;
  border-radius: 5px; background: #2563eb; color: white; cursor: pointer;
  font: 12px/20px -apple-system, sans-serif;
}
.translate-trigger:hover { background: #1d4ed8; }
</style>
