<template>
  <div class="saved-notification" role="status">
    <strong>{{ t('feedback.saved') }}</strong>
    <span>{{ path }}</span>
  </div>
</template>
<script setup lang="ts">
import { t } from '../utils/i18n';
const path = new URLSearchParams(window.location.search).get('path') || '';
</script>
<style scoped>
.saved-notification { margin: 8px; padding: 16px 20px; border-radius: 12px; background: var(--macos-bg-window); color: var(--macos-text-primary); border: 1px solid var(--macos-border-subtle); box-shadow: var(--macos-shadow-popup); display: flex; flex-direction: column; gap: 6px; }
span { overflow-wrap: anywhere; font-size: 12px; user-select: text; }
</style>
