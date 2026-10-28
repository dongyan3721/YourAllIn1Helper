<template>
  <div class="translator-panel">
    <!-- 面板标题与状态栏 -->
    <header class="panel-header">
      <div class="header-info">
        <h2 class="panel-title">{{ t('translator.title') }}</h2>
        <span class="module-number">07</span>
      </div>
      <div class="header-badges">
        <span class="badge active">{{ activeProviderTitle }}</span>
        <span class="badge">{{ config.hotkey }}</span>
      </div>
    </header>

    <div class="panel-content-grid">
      <!-- 左栏：在线即时翻译沙箱 -->
      <section class="sandbox-card macos-card">
        <div class="sandbox-toolbar">
          <div class="lang-controls">
            <select v-model="sourceLang" class="macos-select">
              <option value="auto">{{ t('translator.autoDetect') }}</option>
              <option value="zh-CN">{{ t('translator.langZh') }}</option>
              <option value="en-US">{{ t('translator.langEn') }}</option>
              <option value="ja-JP">{{ t('translator.langJa') }}</option>
            </select>
            <span class="arrow-sep">→</span>
            <select v-model="targetLang" class="macos-select">
              <option value="zh-CN">{{ t('translator.langZh') }}</option>
              <option value="en-US">{{ t('translator.langEn') }}</option>
              <option value="ja-JP">{{ t('translator.langJa') }}</option>
            </select>
          </div>
          <button
            class="macos-btn primary-btn"
            :disabled="isTranslating || !sourceInput.trim()"
            @click="handleTranslate"
          >
            <Icons name="translate" :size="13" />
            <span>{{ isTranslating ? t('translator.translating') : t('translator.title') }}</span>
          </button>
        </div>

        <div class="io-containers">
          <!-- 输入区 -->
          <div class="io-box input-box">
            <div class="io-header">
              <span>{{ t('translator.sourceText') }}</span>
              <button v-if="sourceInput" class="clear-btn" @click="sourceInput = ''">
                <Icons name="close" :size="11" />
              </button>
            </div>
            <textarea
              v-model="sourceInput"
              class="macos-textarea io-textarea"
              :placeholder="t('translator.sourceText')"
              rows="6"
              @keydown.enter.ctrl.prevent="handleTranslate"
              @keydown.enter.meta.prevent="handleTranslate"
            ></textarea>
          </div>

          <!-- 输出区 -->
          <div class="io-box output-box">
            <div class="io-header">
              <span>{{ t('translator.targetText') }}</span>
              <button
                v-if="translatedOutput"
                class="macos-btn mini-btn"
                :class="{ copied: isCopied }"
                @click="copyOutput"
              >
                <Icons :name="isCopied ? 'check' : 'copy'" :size="12" />
                <span>{{ isCopied ? t('translator.copied') : t('translator.copy') }}</span>
              </button>
            </div>
            <div class="io-textarea output-display" :class="{ empty: !translatedOutput }">
              <div v-if="translatedOutput">{{ translatedOutput }}</div>
              <div v-else-if="isTranslating" class="loading-state">
                <span class="spinner"></span>
                <span>{{ t('translator.translating') }}</span>
              </div>
              <div v-else class="empty-text">{{ t('translator.targetText') }}</div>
            </div>
          </div>
        </div>
      </section>

      <!-- 右栏：服务提供商凭据与配置 -->
      <aside class="settings-card macos-card">
        <h3 class="settings-section-title">{{ t('translator.provider') }}</h3>

        <!-- 提供商选择 -->
        <div class="form-group">
          <label class="form-label">{{ t('translator.provider') }}</label>
          <select v-model="config.default_provider_id" class="macos-select full-width" @change="saveConfig">
            <option v-for="p in providers" :key="p.id" :value="p.id">
              {{ p.name }} ({{ p.has_api_key ? t('translator.apiKeyConfigured') : t('translator.apiKeyNotConfigured') }})
            </option>
          </select>
        </div>

        <!-- 当前提供商 API Key 设置 -->
        <div class="form-group">
          <div class="key-header">
            <label class="form-label">{{ t('translator.apiKey') }}</label>
            <span class="status-indicator" :class="{ ok: activeProviderHasKey }">
              {{ activeProviderHasKey ? t('translator.apiKeyConfigured') : t('translator.apiKeyNotConfigured') }}
            </span>
          </div>
          <div class="key-input-row">
            <input
              v-model="inputApiKey"
              type="password"
              class="macos-input key-input"
              :placeholder="
                credentialBackend === 'system_keychain'
                  ? t('translator.apiKeyPlaceholderKeychain')
                  : t('translator.apiKeyPlaceholder')
              "
            />
            <button class="macos-btn mini-btn" :disabled="!inputApiKey.trim()" @click="saveApiKey">
              {{ t('translator.saveApiKey') }}
            </button>
          </div>
          <p class="form-hint">
            {{
              credentialBackend === 'system_keychain'
                ? t('translator.keychainCredentials')
                : t('translator.sessionCredentials')
            }}
          </p>
        </div>

        <div class="form-group">
          <label class="checkbox-label">
            <input v-model="config.mouse_selection_enabled" type="checkbox" @change="saveConfig" />
            <span>{{ t('translator.mouseSelection') }}</span>
          </label>
          <p class="form-hint">{{ t('translator.mouseSelectionHint') }}</p>
        </div>

        <!-- 通用选项 -->
        <h3 class="settings-section-title" style="margin-top: 18px;">常规偏好</h3>
        <div class="form-group">
          <label class="checkbox-label">
            <input :checked="false" type="checkbox" disabled :title="t('translator.pendingOption')" />
            <span>{{ t('translator.autoCopy') }}</span>
          </label>
        </div>
        <div class="form-group">
          <label class="checkbox-label">
            <input :checked="false" type="checkbox" disabled :title="t('translator.pendingOption')" />
            <span>{{ t('translator.history') }}</span>
          </label>
        </div>

        <p class="form-hint">{{ t('translator.pendingOption') }}</p>

        <!-- 自定义大模型服务源 -->
        <div class="custom-providers-section">
          <div class="section-sub-header">
            <h4 class="sub-title">{{ t('translator.customProviders') }}</h4>
            <button class="text-btn" @click="showAddCustom = !showAddCustom">
              + {{ t('translator.addProvider') }}
            </button>
          </div>

          <!-- 添加表单 -->
          <div v-if="showAddCustom" class="add-custom-box">
            <input v-model="newCustom.id" placeholder="唯一 ID (如: ollama)" class="macos-input mini" />
            <input v-model="newCustom.name" placeholder="显示名称 (如: Local Ollama)" class="macos-input mini" />
            <input v-model="newCustom.endpoint_url" placeholder="端点 URL (如: http://localhost:11434/v1/chat/completions)" class="macos-input mini" />
            <input v-model="newCustom.model" placeholder="模型名称 (如: llama3)" class="macos-input mini" />
            <div class="add-actions">
              <button class="macos-btn mini-btn primary" @click="addCustomProvider">确定添加</button>
              <button class="macos-btn mini-btn" @click="showAddCustom = false">取消</button>
            </div>
          </div>
        </div>
      </aside>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import Icons from './Icons.vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';

interface ProviderMeta {
  id: string;
  name: string;
  requires_api_key: boolean;
  has_api_key: boolean;
}

const sourceInput = ref('');
const translatedOutput = ref('');
const isTranslating = ref(false);
const isCopied = ref(false);

const sourceLang = ref('auto');
const targetLang = ref('zh-CN');
const inputApiKey = ref('');
const showAddCustom = ref(false);

const config = ref({
  mouse_selection_enabled: false,
  default_provider_id: 'google',
  target_lang: 'zh-CN',
  source_lang: 'auto',
  auto_copy_result: false,
  history_enabled: false,
  hotkey: 'PrimaryMod+Shift+T',
  popover_width: 420.0,
  popover_height: 340.0,
  custom_providers: [] as Array<{
    id: string;
    name: string;
    endpoint_url: string;
    model?: string;
    temperature?: number;
    is_openai_compatible: boolean;
  }>,
});

const newCustom = ref({
  id: '',
  name: '',
  endpoint_url: '',
  model: '',
});

const providers = ref<ProviderMeta[]>([]);
/** API Key 的实际保存位置（由后端报告，默认按会话内存显示）。 */
const credentialBackend = ref<'session' | 'system_keychain'>('session');
let unlistenStream: (() => void) | null = null;
let currentRequestId = -1;
let submission = 0;
let earlyEvents: any[] = [];
let awaitingRequest = false;
let disposed = false;

const activeProviderTitle = computed(() => {
  const p = providers.value.find((item) => item.id === config.value.default_provider_id);
  return p ? p.name : config.value.default_provider_id;
});

const activeProviderHasKey = computed(() => {
  const p = providers.value.find((item) => item.id === config.value.default_provider_id);
  return p ? p.has_api_key : false;
});

async function loadData() {
  try {
    const cfg = await invokeIpc('translator_get_config');
    if (cfg) {
      config.value = cfg;
      targetLang.value = cfg.target_lang || 'zh-CN';
      sourceLang.value = cfg.source_lang || 'auto';
    }
    const provs = await invokeIpc<ProviderMeta[]>('translator_list_providers');
    if (provs) {
      providers.value = provs;
    }
    const storage = await invokeIpc<{ backend: 'session' | 'system_keychain' }>(
      'translator_credential_storage'
    );
    credentialBackend.value = storage?.backend ?? 'session';
  } catch (e) {
    console.error('加载翻译配置失败:', e);
  }
}

async function handleTranslate() {
  const text = sourceInput.value.trim();
  if (!text) return;

  isTranslating.value = true;
  translatedOutput.value = '';

  const generation = ++submission;
  awaitingRequest = true;
  earlyEvents = [];
  currentRequestId = -1;

  try {
    const res = await invokeIpc('translator_translate', {
      text,
      from: sourceLang.value,
      to: targetLang.value,
      provider_id: config.value.default_provider_id,
      stream: true,
    });
    if (res?.request_id) {
      if (generation !== submission) return;
      currentRequestId = res.request_id;
      awaitingRequest = false;
      const buffered = earlyEvents;
      earlyEvents = [];
      for (const payload of buffered) receiveStream(payload);
    }
  } catch (err: any) {
    if (generation !== submission) return;
    awaitingRequest = false;
    earlyEvents = [];
    isTranslating.value = false;
    translatedOutput.value = `[翻译失败] ${err.message || err}`;
  }
}

async function saveApiKey() {
  const key = inputApiKey.value.trim();
  if (!key) return;

  try {
    await invokeIpc('translator_set_api_key', {
      provider_id: config.value.default_provider_id,
      api_key: key,
    });
    inputApiKey.value = '';
    await loadData();
  } catch (e) {
    console.error('设置 API Key 失败:', e);
  }
}

async function saveConfig() {
  config.value.target_lang = targetLang.value;
  config.value.source_lang = sourceLang.value;
  try {
    await invokeIpc('translator_update_config', config.value);
    await loadData();
  } catch (e) {
    console.error('保存设置失败:', e);
  }
}

async function addCustomProvider() {
  if (!newCustom.value.id || !newCustom.value.name || !newCustom.value.endpoint_url) {
    return;
  }
  config.value.custom_providers.push({
    id: newCustom.value.id.trim(),
    name: newCustom.value.name.trim(),
    endpoint_url: newCustom.value.endpoint_url.trim(),
    model: newCustom.value.model.trim() || undefined,
    is_openai_compatible: true,
  });
  newCustom.value = { id: '', name: '', endpoint_url: '', model: '' };
  showAddCustom.value = false;
  await saveConfig();
}

async function copyOutput() {
  if (!translatedOutput.value) return;
  try {
    await navigator.clipboard.writeText(translatedOutput.value);
    isCopied.value = true;
    setTimeout(() => {
      isCopied.value = false;
    }, 2000);
  } catch (e) {
    console.error('复制失败:', e);
  }
}

function receiveStream(payload: any) {
  if (disposed) return;
  if (awaitingRequest) {
    earlyEvents.push(payload);
    return;
  }
  if (!payload || payload.request_id !== currentRequestId) {
    return;
  }
  const ev = payload.event;
  if (ev.type === 'chunk') {
    translatedOutput.value += ev.payload;
  } else if (ev.type === 'done') {
    translatedOutput.value = ev.payload;
    isTranslating.value = false;
  } else if (ev.type === 'error') {
    isTranslating.value = false;
    translatedOutput.value = `[错误] ${ev.payload}`;
  }
}

onMounted(async () => {
  await loadData();
  if (disposed) return;

  const eventApi = (window as any).__TAURI__?.event;
  if (eventApi?.listen) {
    unlistenStream = await eventApi.listen('translator://stream-chunk', (event: any) => {
      const payload = event.payload;
      receiveStream(payload);
    });
    if (disposed) { unlistenStream?.(); return; }
  }
});

onUnmounted(() => {
  disposed = true;
  ++submission;
  earlyEvents = [];
  if (unlistenStream) {
    unlistenStream();
    unlistenStream = null;
  }
});
</script>

<style scoped>
.translator-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 18px 24px;
  height: 100%;
  box-sizing: border-box;
  overflow-y: auto;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.header-info {
  display: flex;
  align-items: center;
  gap: 10px;
}

.panel-title {
  margin: 0;
  font-size: 20px;
  font-weight: 700;
  color: var(--text-primary);
}

.module-number {
  font-size: 11px;
  font-weight: 700;
  padding: 2px 6px;
  border-radius: 4px;
  background: rgba(0, 122, 255, 0.12);
  color: var(--color-primary);
}

.header-badges {
  display: flex;
  gap: 6px;
}

.badge {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 12px;
  background: var(--bg-hover, rgba(0, 0, 0, 0.05));
  color: var(--text-secondary);
}

.badge.active {
  background: rgba(52, 199, 89, 0.15);
  color: #34c759;
  font-weight: 600;
}

.panel-content-grid {
  display: grid;
  grid-template-columns: 1fr 340px;
  gap: 16px;
  flex: 1;
}

.macos-card {
  background: var(--bg-card, #ffffff);
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.08));
  border-radius: 10px;
  padding: 16px;
  box-sizing: border-box;
}

.sandbox-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.sandbox-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.lang-controls {
  display: flex;
  align-items: center;
  gap: 6px;
}

.arrow-sep {
  color: var(--text-secondary);
  font-weight: 600;
}

.macos-select {
  padding: 5px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color, #d1d1d6);
  background: var(--bg-card);
  color: inherit;
  font-size: 12px;
  outline: none;
}

.macos-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.12));
  background: var(--bg-card);
  font-size: 12px;
  font-weight: 500;
  color: var(--text-primary);
  cursor: pointer;
  transition: all 0.15s ease;
}

.macos-btn.primary-btn {
  background: var(--color-primary, #007aff);
  color: #ffffff;
  border-color: var(--color-primary, #007aff);
}

.macos-btn.mini-btn {
  padding: 3px 8px;
  font-size: 11px;
}

.macos-btn.copied {
  background: #34c759;
  color: #ffffff;
  border-color: #34c759;
}

.io-containers {
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1;
}

.io-box {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
}

.io-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.clear-btn {
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.io-textarea {
  box-sizing: border-box;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border-color, rgba(0, 0, 0, 0.1));
  font-size: 13px;
  line-height: 1.5;
  color: inherit;
  font-family: inherit;
  background: var(--bg-input, #f9f9f9);
}

.output-display {
  min-height: 120px;
  flex: 1;
  background: rgba(0, 0, 0, 0.02);
  user-select: text;
}

.output-display.empty {
  display: flex;
  align-items: center;
  justify-content: center;
}

.empty-text {
  color: var(--text-secondary);
  font-style: italic;
}

.settings-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.settings-section-title {
  margin: 0;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-primary);
  border-bottom: 1px solid var(--border-color);
  padding-bottom: 6px;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.form-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.full-width {
  width: 100%;
}

.key-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.status-indicator {
  font-size: 10px;
  font-weight: 600;
  color: #ff9500;
}

.status-indicator.ok {
  color: #34c759;
}

.key-input-row {
  display: flex;
  gap: 6px;
}

.key-input {
  flex: 1;
}

.macos-input {
  padding: 5px 8px;
  border-radius: 6px;
  border: 1px solid var(--border-color, #d1d1d6);
  background: var(--bg-card);
  color: inherit;
  font-size: 12px;
  outline: none;
}

.form-hint {
  margin: 2px 0 0;
  font-size: 10px;
  color: var(--text-secondary);
  line-height: 1.4;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-primary);
  cursor: pointer;
}

.custom-providers-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 10px;
}

.section-sub-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.sub-title {
  margin: 0;
  font-size: 12px;
  color: var(--text-secondary);
}

.text-btn {
  background: none;
  border: none;
  font-size: 11px;
  color: var(--color-primary);
  cursor: pointer;
  padding: 0;
}

.add-custom-box {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.03);
}

.macos-input.mini {
  font-size: 11px;
  padding: 4px 6px;
}

.add-actions {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}

.mini-btn.primary {
  background: var(--color-primary);
  color: #ffffff;
}
</style>
