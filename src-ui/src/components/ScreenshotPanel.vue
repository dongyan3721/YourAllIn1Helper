<template>
  <div class="shot-panel">
    <header class="shot-heading">
      <div>
        <h2>{{ t('screenshot.panel.title') }}</h2>
        <p>{{ t('screenshot.panel.subtitle') }}</p>
      </div>
      <div class="heading-actions">
        <select v-if="monitors.length > 1" v-model="monitorId" class="shot-input monitor-select">
          <option v-for="m in monitors" :key="m.id" :value="m.id">
            {{ m.name }}{{ m.is_primary ? ` (${t('screenshot.panel.primary')})` : '' }}
          </option>
        </select>
        <button class="macos-button" :disabled="busy || !status?.permission.granted" @click="startCapture('ocr')">
          <Icons name="text" :size="12" />
          <span>{{ t('screenshot.panel.startOcrBtn') }}</span>
        </button>
        <button class="macos-button primary" :disabled="busy || !status?.permission.granted" @click="startCapture('capture')">
          <Icons name="camera" :size="12" />
          <span>{{ t('screenshot.panel.startBtn') }}</span>
        </button>
      </div>
    </header>

    <div v-if="notice" class="shot-notice" :class="notice.kind" role="status">
      <span>{{ notice.text }}</span>
      <button class="notice-close" @click="notice = null"><Icons name="close" :size="12" /></button>
    </div>

    <!-- 权限与快捷键真实状态 -->
    <section class="shot-card">
      <div class="status-grid">
        <div class="status-item">
          <span class="status-dot" :class="{ ok: status?.permission.granted }"></span>
          <div>
            <strong>{{ t('screenshot.panel.permissionLabel') }}</strong>
            <span v-if="status?.permission.granted">{{ t('screenshot.panel.permissionGranted') }}</span>
            <template v-else>
              <span class="warn">{{ status?.permission.message || t('screenshot.panel.permissionUnknown') }}</span>
              <span v-if="status?.permission.guidance" class="hint">{{ status.permission.guidance }}</span>
            </template>
          </div>
        </div>
        <div class="status-item">
          <span class="status-dot" :class="{ ok: shortcutOk }"></span>
          <div>
            <strong>{{ t('screenshot.panel.shortcutLabel') }}</strong>
            <span v-if="shortcutOk">{{ status?.shortcut.display_label }}</span>
            <span v-else class="warn">{{ status?.native_shortcut_error || t('screenshot.panel.shortcutMissing') }}</span>
          </div>
        </div>
        <div class="status-item">
          <span class="status-dot" :class="{ ok: !!status?.session }"></span>
          <div>
            <strong>{{ t('screenshot.panel.sessionLabel') }}</strong>
            <span>{{ status?.session ? t('screenshot.panel.sessionActive', { monitor: status.session.monitor_name }) : t('screenshot.panel.sessionIdle') }}</span>
            <button v-if="status?.session" class="link-btn" @click="cancelSession">{{ t('screenshot.panel.cancelSession') }}</button>
          </div>
        </div>
        <div class="status-item">
          <span class="status-dot" :class="{ ok: (status?.pins.length ?? 0) > 0 }"></span>
          <div>
            <strong>{{ t('screenshot.panel.pinsLabel') }}</strong>
            <span>{{ t('screenshot.panel.pinsCount', { count: status?.pins.length ?? 0, max: status?.settings.max_pins ?? 0 }) }}</span>
          </div>
        </div>
      </div>

      <div v-if="status?.last_error" class="result-line error">
        <strong>{{ t('screenshot.panel.lastError') }}</strong>
        <span>{{ status.last_error.message }}</span>
        <span v-if="status.last_error.guidance" class="hint">{{ status.last_error.guidance }}</span>
      </div>
      <div v-else-if="status?.last_export" class="result-line">
        <strong>{{ t('screenshot.panel.lastResult') }}</strong>
        <span>{{ describeExport(status.last_export) }}</span>
      </div>
    </section>

    <!-- 贴图列表 -->
    <section class="shot-card">
      <div class="card-heading">
        <h3>{{ t('screenshot.panel.pinsTitle') }}</h3>
        <button class="macos-button" :disabled="!status?.pins.length" @click="closeAll">
          <Icons name="close" :size="12" />
          <span>{{ t('screenshot.panel.closeAllPins') }}</span>
        </button>
      </div>
      <table class="macos-table">
        <thead>
          <tr>
            <th>{{ t('screenshot.panel.colPin') }}</th>
            <th>{{ t('screenshot.panel.colSize') }}</th>
            <th>{{ t('screenshot.panel.colCreated') }}</th>
            <th>{{ t('screenshot.panel.colActions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="!status?.pins.length">
            <td colspan="4" class="empty">{{ t('screenshot.panel.noPins') }}</td>
          </tr>
          <tr v-for="pin in status?.pins ?? []" :key="pin.pin_id">
            <td><code>{{ pin.pin_id.slice(0, 8) }}</code></td>
            <td>{{ pin.pixel_width }} × {{ pin.pixel_height }} px</td>
            <td>{{ formatTime(pin.created_at) }}</td>
            <td class="actions">
              <button class="macos-button" @click="pinAction('copy', pin.pin_id)">{{ t('screenshot.pin.copy') }}</button>
              <button class="macos-button" @click="pinAction('save', pin.pin_id)">{{ t('screenshot.pin.save') }}</button>
              <button class="macos-button danger" @click="pinAction('close', pin.pin_id)">{{ t('screenshot.pin.close') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- 设置 -->
    <section class="shot-card">
      <div class="card-heading">
        <h3>{{ t('screenshot.panel.settingsTitle') }}</h3>
        <button class="macos-button" :disabled="saving || !form" @click="saveSettings">
          <Icons name="check" :size="12" />
          <span>{{ saving ? t('screenshot.panel.saving') : t('screenshot.panel.saveSettings') }}</span>
        </button>
      </div>
      <div v-if="form" class="settings-grid">
        <label class="field wide">
          <span>{{ t('screenshot.panel.saveDirectory') }}</span>
          <input v-model="form.save_directory" class="shot-input" type="text" :placeholder="status?.save_directory" spellcheck="false" />
        </label>
        <label class="field">
          <span>{{ t('screenshot.panel.filePrefix') }}</span>
          <input v-model="form.file_name_prefix" class="shot-input" type="text" spellcheck="false" />
        </label>
        <label class="field">
          <span>{{ t('screenshot.panel.maxPins') }}</span>
          <input v-model.number="form.max_pins" class="shot-input" type="number" min="1" max="64" />
        </label>
        <label class="field">
          <span>{{ t('screenshot.panel.defaultColor') }}</span>
          <input v-model="form.default_color" class="shot-input color-input" type="color" />
        </label>
        <label class="field">
          <span>{{ t('screenshot.panel.defaultWidth') }}</span>
          <input v-model.number="form.default_stroke_width" class="shot-input" type="number" min="1" max="32" />
        </label>
        <label class="field">
          <span>{{ t('screenshot.panel.pinOpacity', { value: Math.round((form.default_pin_opacity || 1) * 100) }) }}</span>
          <input v-model.number="form.default_pin_opacity" type="range" min="0.1" max="1" step="0.1" />
        </label>
        <label class="check"><input v-model="form.copy_after_save" type="checkbox" />{{ t('screenshot.panel.copyAfterSave') }}</label>
        <label class="check"><input v-model="flags.show_magnifier" type="checkbox" />{{ t('screenshot.panel.magnifier') }}</label>
        <label class="check"><input v-model="flags.enable_smart_region_detection" type="checkbox" />{{ t('screenshot.panel.smartRegions') }}</label>
        <label class="check"><input v-model="flags.smart_region_wheel_expand" type="checkbox" />{{ t('screenshot.panel.wheelExpand') }}</label>
        <label class="check"><input v-model="flags.enable_manual_box_selection" type="checkbox" />{{ t('screenshot.panel.manualSelection') }}</label>
      </div>
    </section>

    <!-- 能力边界与操作说明 -->
    <section class="shot-card">
      <h3>{{ t('screenshot.panel.capabilityTitle') }}</h3>
      <table class="macos-table">
        <tbody>
          <tr v-for="row in capabilityRows" :key="row.key">
            <td><strong>{{ t(`screenshot.panel.cap.${row.key}`) }}</strong></td>
            <td><span class="type-pill" :class="row.ready ? 'image' : 'text'">{{ row.ready ? t('screenshot.panel.capReady') : t('screenshot.panel.capPending') }}</span></td>
            <td>{{ t(`screenshot.panel.cap.${row.key}Desc`) }}</td>
          </tr>
        </tbody>
      </table>
      <h3 class="guide-title">{{ t('screenshot.panel.guideTitle') }}</h3>
      <ul class="guide-list">
        <li v-for="key in guideKeys" :key="key">{{ t(`screenshot.panel.guide.${key}`) }}</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
/**
 * 主控台「增强截图」页：真实权限/快捷键状态、开始截图、贴图管理与设置。
 * 自定义 ONNX 模型仍未实现，能力表里保持待开发。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';
import {
  screenshotApi,
  type ExportOutcome,
  type MonitorInfo,
  type OverlayFlags,
  type ScreenshotSettings,
  type ScreenshotStatus,
} from '../api/screenshot';

const status = ref<ScreenshotStatus | null>(null);
const monitors = ref<MonitorInfo[]>([]);
const monitorId = ref<number | null>(null);
const form = ref<ScreenshotSettings | null>(null);
const flags = reactive<OverlayFlags>({
  enable_manual_box_selection: true,
  enable_smart_region_detection: true,
  smart_region_wheel_expand: true,
  show_magnifier: true,
});
const busy = ref(false);
const saving = ref(false);
const notice = ref<{ text: string; kind: 'ok' | 'error' } | null>(null);

const shortcutOk = computed(() => !!status.value?.shortcut.registered && !status.value?.native_shortcut_error);

const capabilityRows = [
  { key: 'capture', ready: true },
  { key: 'manual', ready: true },
  { key: 'windowRegions', ready: true },
  { key: 'annotate', ready: true },
  { key: 'pin', ready: true },
  { key: 'uiElements', ready: true },
  { key: 'ocr', ready: true },
  { key: 'scroll', ready: true },
  { key: 'gif', ready: true },
  { key: 'onnx', ready: false },
];
const guideKeys = ['select', 'smart', 'adjust', 'tools', 'export', 'color', 'ocr', 'pin'];

function formatTime(ms: number) {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
}

function describeExport(o: ExportOutcome): string {
  const size = `${o.width} × ${o.height}`;
  if (o.action === 'pin') return t('screenshot.panel.resultPin', { size });
  if (o.action === 'copy') return t('screenshot.panel.resultCopy', { size });
  const base = t('screenshot.panel.resultSave', { size, path: o.saved_path ?? '' });
  if (o.clipboard_error) return `${base} · ${t('screenshot.panel.resultCopyFailed', { error: o.clipboard_error })}`;
  return o.copied ? `${base} · ${t('screenshot.panel.resultAlsoCopied')}` : base;
}

async function refresh() {
  try {
    status.value = await screenshotApi.status();
  } catch (err) {
    console.error('刷新截图状态失败:', err);
  }
}

async function loadConfig() {
  try {
    const [config, list] = await Promise.all([screenshotApi.getConfig(), screenshotApi.listMonitors().catch(() => [])]);
    form.value = { ...config.settings, save_directory: config.settings.save_directory ?? '' };
    Object.assign(flags, config.overlay);
    monitors.value = list;
    monitorId.value = list.find((m) => m.is_primary)?.id ?? list[0]?.id ?? null;
  } catch (err: any) {
    notice.value = { text: err?.message || String(err), kind: 'error' };
  }
}

async function startCapture(intent: 'capture' | 'ocr') {
  busy.value = true;
  try {
    await screenshotApi.start(monitors.value.length > 1 ? monitorId.value : null, intent);
  } catch (err: any) {
    notice.value = { text: err?.message || String(err), kind: 'error' };
  } finally {
    busy.value = false;
    await refresh();
  }
}

async function cancelSession() {
  await screenshotApi.cancel().catch(() => undefined);
  await refresh();
}

async function closeAll() {
  try {
    await screenshotApi.closeAllPins();
  } catch (err: any) {
    notice.value = { text: err?.message || String(err), kind: 'error' };
  }
  await refresh();
}

async function pinAction(action: 'copy' | 'save' | 'close', pinId: string) {
  try {
    if (action === 'copy') {
      await screenshotApi.copyPin(pinId);
      notice.value = { text: t('screenshot.pin.copied'), kind: 'ok' };
    } else if (action === 'save') {
      const res = await screenshotApi.savePin(pinId);
      notice.value = { text: t('screenshot.pin.saved', { path: res.saved_path }), kind: 'ok' };
    } else {
      await screenshotApi.closePin(pinId);
    }
  } catch (err: any) {
    notice.value = { text: err?.message || String(err), kind: 'error' };
  }
  await refresh();
}

async function saveSettings() {
  if (!form.value) return;
  saving.value = true;
  try {
    const patch: Partial<ScreenshotSettings> = {
      ...form.value,
      save_directory: form.value.save_directory?.trim() ? form.value.save_directory.trim() : null,
    };
    const saved = await screenshotApi.updateSettings(patch, { ...flags });
    form.value = { ...saved.settings, save_directory: saved.settings.save_directory ?? '' };
    Object.assign(flags, saved.overlay);
    notice.value = { text: t('screenshot.panel.settingsSaved'), kind: 'ok' };
  } catch (err: any) {
    notice.value = { text: err?.message || String(err), kind: 'error' };
  } finally {
    saving.value = false;
    await refresh();
  }
}

let timer: ReturnType<typeof setInterval> | undefined;
onMounted(async () => {
  await Promise.all([refresh(), loadConfig()]);
  timer = setInterval(refresh, 1500);
});
onUnmounted(() => {
  if (timer) clearInterval(timer);
});
</script>

<style scoped>
.shot-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.shot-heading {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}
.shot-heading h2 {
  margin: 0 0 4px;
  font-size: 17px;
  color: var(--macos-text-primary);
}
.shot-heading p {
  margin: 0;
  font-size: 12px;
  color: var(--macos-text-secondary);
}
.heading-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}
.shot-card {
  padding: 14px 16px;
  border-radius: var(--macos-radius-lg);
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
}
.shot-card h3 {
  margin: 0 0 10px;
  font-size: 13px;
  color: var(--macos-text-primary);
}
.card-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 10px;
}
.card-heading h3 { margin: 0; }
.status-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
}
.status-item {
  display: flex;
  gap: 10px;
  align-items: flex-start;
}
.status-item > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
  color: var(--macos-text-secondary);
}
.status-item strong { color: var(--macos-text-primary); }
.status-dot {
  flex: none;
  width: 8px;
  height: 8px;
  margin-top: 4px;
  border-radius: 50%;
  background: var(--macos-text-tertiary);
}
.status-dot.ok { background: var(--macos-success); }
.warn { color: var(--macos-danger); }
.hint { color: var(--macos-text-tertiary); }
.link-btn {
  align-self: flex-start;
  padding: 0;
  border: none;
  background: none;
  color: var(--macos-accent);
  font-size: 12px;
  cursor: pointer;
}
.result-line {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--macos-border-separator);
  font-size: 12px;
  color: var(--macos-text-secondary);
  word-break: break-all;
}
.result-line.error span:first-of-type { color: var(--macos-danger); }
.shot-notice {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 12px;
  border-radius: var(--macos-radius-md);
  font-size: 12px;
  background: var(--macos-success-subtle);
  color: var(--macos-text-primary);
  word-break: break-all;
}
.shot-notice.error { background: var(--macos-danger-subtle); }
.notice-close {
  border: none;
  background: none;
  color: var(--macos-text-secondary);
  cursor: pointer;
}
.settings-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 10px 14px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
  color: var(--macos-text-secondary);
}
.field.wide { grid-column: 1 / -1; }
.check {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--macos-text-primary);
}
.shot-input {
  padding: 5px 8px;
  border-radius: var(--macos-radius-sm);
  border: 1px solid var(--macos-border-subtle);
  background: var(--macos-bg-input);
  color: var(--macos-text-primary);
  font-size: 12px;
}
.color-input { width: 56px; height: 28px; padding: 2px; }
.monitor-select { max-width: 220px; }
.actions { display: flex; gap: 6px; }
.empty { text-align: center; color: var(--macos-text-tertiary); }
.guide-title { margin-top: 14px !important; }
.guide-list {
  margin: 0;
  padding-left: 18px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--macos-text-secondary);
}
</style>
