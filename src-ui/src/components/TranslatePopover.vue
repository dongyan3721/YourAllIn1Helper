<template>
  <div class="translate-popover-container" :class="{ 'is-pinned': isPinned }">
    <!-- 顶部拖拽栏与控制条 -->
    <header class="popover-header" data-tauri-drag-region>
      <div class="header-left">
        <Icons name="translate" :size="15" class="header-icon" />
        <span class="header-title">{{ t('translator.title') }}</span>
        <span class="provider-badge">{{ currentProviderName }}</span>
      </div>
      <div class="header-right">
        <!-- 语言对切换 -->
        <div class="lang-selector">
          <select v-model="selectedSourceLang" class="lang-select" @change="triggerTranslate">
            <option value="auto">{{ t('translator.autoDetect') }}</option>
            <option value="zh-CN">{{ t('translator.langZh') }}</option>
            <option value="en-US">{{ t('translator.langEn') }}</option>
            <option value="ja-JP">{{ t('translator.langJa') }}</option>
          </select>
          <span class="lang-arrow">→</span>
          <select v-model="selectedTargetLang" class="lang-select" @change="triggerTranslate">
            <option value="zh-CN">{{ t('translator.langZh') }}</option>
            <option value="en-US">{{ t('translator.langEn') }}</option>
            <option value="ja-JP">{{ t('translator.langJa') }}</option>
          </select>
        </div>

        <!-- 置顶与关闭 -->
        <button
          class="icon-btn"
          :class="{ active: isPinned }"
          :title="isPinned ? t('translator.unpin') : t('translator.pin')"
          @click="togglePin"
        >
          <Icons :name="isPinned ? 'pin-fill' : 'pin'" :size="13" />
        </button>
        <button class="icon-btn close-btn" :title="t('translator.close')" @click="closePopover">
          <Icons name="close" :size="13" />
        </button>
      </div>
    </header>

    <!-- 核心内容区 -->
    <main class="popover-body">
      <!-- 权限警告 -->
      <div v-if="permissionDenied" class="alert-box warning-alert">
        <Icons name="power" :size="14" class="alert-icon" />
        <div class="alert-content">
          <strong>{{ t('translator.permissionDenied') }}</strong>
          <p>{{ permissionGuidance || t('translator.permissionGuidance') }}</p>
        </div>
      </div>

      <!-- API Key 缺失警告 -->
      <div v-else-if="missingApiKey" class="alert-box error-alert">
        <Icons name="terminal" :size="14" class="alert-icon" />
        <div class="alert-content">
          <p>{{ t('translator.missingApiKey') }}</p>
          <button class="link-btn" @click="openDashboardSettings">{{ t('translator.openSettings') }}</button>
        </div>
      </div>

      <!-- 原文展示与编辑区 -->
      <section class="section-box source-section">
        <div class="section-label">
          <span>{{ t('translator.sourceText') }}</span>
          <button v-if="sourceText" class="text-action-btn" @click="triggerTranslate">
            {{ t('translator.retranslate') }}
          </button>
        </div>
        <textarea
          v-model="sourceText"
          class="source-textarea"
          :placeholder="t('translator.noSelection')"
          rows="3"
          @keydown.enter.exact.prevent="triggerTranslate"
        ></textarea>
      </section>

      <!-- 译文展示区 -->
      <section class="section-box target-section">
        <div class="section-label">
          <span>{{ t('translator.targetText') }}</span>
          <div class="target-actions">
            <span v-if="isTranslating" class="translating-indicator">
              <span class="spinner-dot"></span>
              {{ t('translator.translating') }}
            </span>
            <button
              v-if="translatedText"
              class="copy-btn"
              :class="{ copied: isCopied }"
              @click="copyTranslation"
            >
              <Icons :name="isCopied ? 'check' : 'copy'" :size="12" />
              <span>{{ isCopied ? t('translator.copied') : t('translator.copy') }}</span>
            </button>
          </div>
        </div>
        <div class="target-content" :class="{ 'is-empty': !translatedText && !isTranslating }">
          <div v-if="translatedText" class="rendered-text">{{ translatedText }}</div>
          <div v-else-if="isTranslating" class="loading-placeholder">
            <span class="loading-bar"></span>
          </div>
          <div v-else class="empty-hint">{{ t('translator.translating') }}</div>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import Icons from './Icons.vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';

interface StreamChunkEvent {
  request_id: number;
  event: {
    type: 'chunk' | 'done' | 'error';
    payload: string;
  };
}

const sourceText = ref('');
const translatedText = ref('');
const isTranslating = ref(false);
const isCopied = ref(false);
const isPinned = ref(true);
const permissionDenied = ref(false);
const permissionGuidance = ref('');
const missingApiKey = ref(false);

const selectedSourceLang = ref('auto');
const selectedTargetLang = ref('zh-CN');
const currentProviderId = ref('google');
const currentProviderName = computed(() => {
  if (currentProviderId.value === 'google') return t('translator.providerGoogle');
  if (currentProviderId.value === 'deepl') return t('translator.providerDeepL');
  if (currentProviderId.value === 'openai') return t('translator.providerOpenAI');
  return currentProviderId.value;
});

let unlistenStream: (() => void) | null = null;
let activeRequestId = 0;

async function loadConfig() {
  try {
    const cfg = await invokeIpc('translator_get_config');
    if (cfg) {
      currentProviderId.value = cfg.default_provider_id || 'google';
      selectedTargetLang.value = cfg.target_lang || 'zh-CN';
      selectedSourceLang.value = cfg.source_lang || 'auto';
    }
  } catch (e) {
    console.error('读取划词翻译配置失败:', e);
  }
}

async function extractAndTranslate() {
  permissionDenied.value = false;
  missingApiKey.value = false;

  try {
    const sel = await invokeIpc('translator_extract_selection');
    if (sel?.permission_denied) {
      permissionDenied.value = true;
      permissionGuidance.value = sel.guidance || '';
      return;
    }
    if (sel?.has_selection && sel.text) {
      sourceText.value = sel.text;
      await triggerTranslate();
    }
  } catch (err: any) {
    console.error('取词异常:', err);
  }
}

async function triggerTranslate() {
  const text = sourceText.value.trim();
  if (!text) return;

  isTranslating.value = true;
  translatedText.value = '';
  missingApiKey.value = false;

  try {
    const res = await invokeIpc('translator_translate', {
      text,
      from: selectedSourceLang.value,
      to: selectedTargetLang.value,
      provider_id: currentProviderId.value,
      stream: true,
    });
    if (res?.request_id) {
      activeRequestId = res.request_id;
    }
  } catch (err: any) {
    isTranslating.value = false;
    const msg = err.message || String(err);
    if (msg.includes('未配置') || msg.includes('API Key')) {
      missingApiKey.value = true;
    } else {
      translatedText.value = `[翻译错误] ${msg}`;
    }
  }
}

async function copyTranslation() {
  if (!translatedText.value) return;
  try {
    await navigator.clipboard.writeText(translatedText.value);
    isCopied.value = true;
    setTimeout(() => {
      isCopied.value = false;
    }, 2000);
  } catch (e) {
    console.error('复制失败:', e);
  }
}

async function togglePin() {
  isPinned.value = !isPinned.value;
  try {
    await invokeIpc('kernel_set_always_on_top', {
      label: 'translate-popover',
      always_on_top: isPinned.value,
    });
  } catch (e) {
    // 忽略微内核非原生置顶提示
  }
}

async function closePopover() {
  try {
    await invokeIpc('translator_close_popover');
  } catch (e) {
    window.close();
  }
}

function openDashboardSettings() {
  invokeIpc('kernel_open_window', {
    spec: {
      label: 'dashboard',
      route: '/overview',
      title: 'Dashboard',
    },
  }).catch(() => {});
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    closePopover();
  }
}

onMounted(async () => {
  window.addEventListener('keydown', handleKeydown);
  await loadConfig();

  // 监听 Tauri 原生流式事件
  const eventApi = (window as any).__TAURI__?.event;
  if (eventApi?.listen) {
    unlistenStream = await eventApi.listen('translator://stream-chunk', (event: { payload: StreamChunkEvent }) => {
      const payload = event.payload;
      if (!payload || (activeRequestId > 0 && payload.request_id !== activeRequestId)) {
        return; // 丢弃旧请求或迟到分片
      }

      const ev = payload.event;
      if (ev.type === 'chunk') {
        translatedText.value += ev.payload;
      } else if (ev.type === 'done') {
        translatedText.value = ev.payload;
        isTranslating.value = false;
      } else if (ev.type === 'error') {
        isTranslating.value = false;
        if (ev.payload.includes('未配置') || ev.payload.includes('API Key')) {
          missingApiKey.value = true;
        } else {
          translatedText.value = `[错误] ${ev.payload}`;
        }
      }
    });
  }

  // 首次打开自动提取焦点选区并翻译
  await extractAndTranslate();
});

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown);
  if (unlistenStream) {
    unlistenStream();
    unlistenStream = null;
  }
});
</script>

<style scoped>
.translate-popover-container {
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  background: var(--bg-card, rgba(255, 255, 255, 0.88));
  backdrop-filter: blur(24px) saturate(180%);
  -webkit-backdrop-filter: blur(24px) saturate(180%);
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.08));
  border-radius: 10px;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
  color: var(--text-primary, #1c1c1e);
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.16);
  user-select: none;
}

.popover-header {
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-color, rgba(0, 0, 0, 0.06));
  background: rgba(0, 0, 0, 0.02);
  cursor: grab;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 6px;
}

.header-icon {
  color: var(--color-primary, #007aff);
}

.header-title {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: -0.2px;
}

.provider-badge {
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 4px;
  background: rgba(0, 122, 255, 0.1);
  color: var(--color-primary, #007aff);
  font-weight: 500;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 6px;
}

.lang-selector {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
}

.lang-select {
  font-size: 11px;
  padding: 2px 4px;
  border-radius: 4px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.12));
  background: var(--bg-input, rgba(255, 255, 255, 0.7));
  color: inherit;
  outline: none;
}

.lang-arrow {
  color: var(--text-secondary, #8e8e93);
  font-weight: 600;
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  border-radius: 4px;
  color: var(--text-secondary, #8e8e93);
  cursor: pointer;
  transition: all 0.15s ease;
}

.icon-btn:hover {
  background: rgba(0, 0, 0, 0.06);
  color: var(--text-primary, #1c1c1e);
}

.icon-btn.active {
  color: var(--color-primary, #007aff);
}

.close-btn:hover {
  background: rgba(255, 59, 48, 0.15);
  color: #ff3b30;
}

.popover-body {
  flex: 1;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  overflow-y: auto;
}

.alert-box {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  font-size: 11px;
}

.warning-alert {
  background: rgba(255, 149, 0, 0.12);
  color: #c97100;
  border: 1px solid rgba(255, 149, 0, 0.25);
}

.error-alert {
  background: rgba(255, 59, 48, 0.1);
  color: #d70015;
  border: 1px solid rgba(255, 59, 48, 0.2);
}

.alert-content p {
  margin: 2px 0 0;
  line-height: 1.4;
}

.link-btn {
  margin-top: 4px;
  padding: 2px 6px;
  background: rgba(0, 122, 255, 0.12);
  border: 1px solid rgba(0, 122, 255, 0.3);
  border-radius: 4px;
  color: var(--color-primary, #007aff);
  font-size: 10px;
  cursor: pointer;
}

.section-box {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.section-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary, #8e8e93);
}

.text-action-btn {
  background: none;
  border: none;
  font-size: 10px;
  color: var(--color-primary, #007aff);
  cursor: pointer;
  padding: 0;
}

.source-textarea {
  width: 100%;
  box-sizing: border-box;
  padding: 6px 8px;
  border-radius: 6px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.1));
  background: var(--bg-input, rgba(255, 255, 255, 0.6));
  font-size: 12px;
  line-height: 1.4;
  color: inherit;
  resize: vertical;
  outline: none;
}

.source-textarea:focus {
  border-color: var(--color-primary, #007aff);
  background: var(--bg-input-focus, #ffffff);
}

.target-content {
  min-height: 70px;
  padding: 8px 10px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.03);
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.06));
  font-size: 13px;
  line-height: 1.5;
  user-select: text;
}

.target-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.translating-indicator {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  color: var(--color-primary, #007aff);
}

.spinner-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-primary, #007aff);
  animation: pulse 1s infinite alternate;
}

@keyframes pulse {
  0% { transform: scale(0.6); opacity: 0.4; }
  100% { transform: scale(1.1); opacity: 1; }
}

.copy-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.12));
  background: var(--bg-card, rgba(255, 255, 255, 0.8));
  font-size: 10px;
  color: var(--text-primary, #1c1c1e);
  cursor: pointer;
  transition: all 0.15s ease;
}

.copy-btn.copied {
  background: #34c759;
  color: white;
  border-color: #34c759;
}

.empty-hint {
  color: var(--text-secondary, #8e8e93);
  font-size: 12px;
  font-style: italic;
}
</style>
