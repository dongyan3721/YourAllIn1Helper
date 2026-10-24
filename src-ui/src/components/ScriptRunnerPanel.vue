<template>
  <div class="script-runner">
    <p v-if="error" class="script-error" role="alert">{{ error }}</p>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('scriptRunner.title') }}</span>
          <span class="card-desc">{{ t('scriptRunner.desc') }}</span>
        </div>
        <div class="toolbar-actions">
          <button class="macos-button" @click="refreshAll(true)">
            <Icons name="refresh" size="12" />
            <span>{{ t('scriptRunner.refresh') }}</span>
          </button>
          <button class="macos-button primary" @click="openCreate">
            <Icons name="terminal" size="12" />
            <span>{{ t('scriptRunner.newScript') }}</span>
          </button>
        </div>
      </div>

      <div class="probe-grid">
        <div
          v-for="item in interpreters"
          :key="item.script_type"
          class="probe-item"
          :class="{ unavailable: !item.available }"
        >
          <div class="probe-status-dot" :class="{ ready: item.available }"></div>
          <div class="probe-info">
            <span class="probe-name">{{ typeLabel(item.script_type) }}</span>
            <span class="probe-path">
              {{
                item.available
                  ? item.resolved_path || item.version_summary || t('scriptRunner.interpreterReady')
                  : item.supported
                  ? item.install_hint || item.unavailable_reason || t('scriptRunner.interpreterMissing')
                  : item.unavailable_reason || t('scriptRunner.interpreterUnsupported')
              }}
            </span>
          </div>
          <span class="probe-badge">
            {{
              !item.supported
                ? t('scriptRunner.interpreterUnsupported')
                : item.available
                ? t('scriptRunner.interpreterAvailable')
                : t('scriptRunner.interpreterUnavailable')
            }}
          </span>
        </div>
      </div>

      <div class="env-bar">
        <span class="env-title">{{ t('scriptRunner.envTitle') }}</span>
        <span class="env-pill" :class="{ ready: environment?.clipboard }">
          AIO_CLIPBOARD · {{ environment?.clipboard ? t('scriptRunner.envAvailable') : t('scriptRunner.envUnavailable') }}
        </span>
        <span class="env-pill" :class="{ ready: environment?.selection }">
          AIO_SELECTION · {{ environment?.selection ? t('scriptRunner.envAvailable') : t('scriptRunner.envUnavailable') }}
        </span>
        <span class="env-detail">{{ environment?.detail || t('scriptRunner.envNoAdapter') }}</span>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('scriptRunner.scriptsTitle') }}</span>
          <span class="card-desc">{{ t('scriptRunner.scriptsDesc') }}</span>
        </div>
      </div>
      <div class="table-container">
        <table class="macos-table">
          <thead>
            <tr>
              <th>{{ t('scriptRunner.colName') }}</th>
              <th>{{ t('scriptRunner.colType') }}</th>
              <th>{{ t('scriptRunner.colSource') }}</th>
              <th>{{ t('scriptRunner.colArgs') }}</th>
              <th>{{ t('scriptRunner.colHotkey') }}</th>
              <th>{{ t('scriptRunner.colTimeout') }}</th>
              <th>{{ t('scriptRunner.colTray') }}</th>
              <th class="actions-col">{{ t('scriptRunner.colActions') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="scripts.length === 0">
              <td colspan="8" class="empty-cell">{{ t('scriptRunner.emptyScripts') }}</td>
            </tr>
            <tr v-for="script in scripts" :key="script.id">
              <td>
                <strong>{{ script.name }}</strong>
                <div v-if="script.hotkey && !script.hotkey_active" class="inline-warning">
                  {{ t('scriptRunner.hotkeyInactive') }}
                </div>
              </td>
              <td><span class="type-pill">{{ typeLabel(script.script_type) }}</span></td>
              <td>
                <span class="source-pill" :class="{ file: !script.is_inline }">
                  {{ script.is_inline ? t('scriptRunner.inline') : t('scriptRunner.file') }}
                </span>
                <div class="source-preview" :title="script.content_or_path">
                  {{ previewText(script.content_or_path) }}
                </div>
              </td>
              <td class="mono-cell" :title="script.args.join(' ')">
                {{ script.args.length ? script.args.join(' ') : t('scriptRunner.none') }}
              </td>
              <td>
                <code v-if="script.hotkey" class="hotkey-pill">{{ script.hotkey }}</code>
                <span v-else class="muted">{{ t('scriptRunner.noHotkey') }}</span>
              </td>
              <td>{{ t('scriptRunner.secs', { count: script.timeout_secs }) }}</td>
              <td>{{ script.show_in_tray ? t('scriptRunner.trayYes') : t('scriptRunner.trayNo') }}</td>
              <td>
                <div class="btn-group">
                  <button class="macos-button primary" @click="runScript(script)">
                    <Icons name="play" size="11" />
                    <span>{{ t('scriptRunner.run') }}</span>
                  </button>
                  <button class="macos-button" @click="openEdit(script)">
                    <Icons name="text" size="11" />
                    <span>{{ t('scriptRunner.edit') }}</span>
                  </button>
                  <button class="macos-button danger" @click="deleteScript(script)">
                    <Icons name="trash" size="11" />
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('scriptRunner.runsTitle') }}</span>
          <span class="card-desc">{{ t('scriptRunner.runsDesc') }}</span>
        </div>
        <button class="macos-button" @click="refreshRuns">
          <Icons name="refresh" size="12" />
        </button>
      </div>
      <div class="table-container">
        <table class="macos-table">
          <thead>
            <tr>
              <th>{{ t('scriptRunner.colScript') }}</th>
              <th>{{ t('scriptRunner.colStatus') }}</th>
              <th>{{ t('scriptRunner.colExit') }}</th>
              <th>{{ t('scriptRunner.colStarted') }}</th>
              <th>{{ t('scriptRunner.colDuration') }}</th>
              <th class="actions-col">{{ t('scriptRunner.colRunActions') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="runs.length === 0">
              <td colspan="6" class="empty-cell">{{ t('scriptRunner.emptyRuns') }}</td>
            </tr>
            <tr v-for="run in runs" :key="run.run_id">
              <td>
                <strong>{{ run.script_name }}</strong>
                <div class="run-id">{{ run.run_id }}</div>
              </td>
              <td>
                <span class="run-status" :class="run.status">{{ statusLabel(run.status) }}</span>
              </td>
              <td>{{ run.exit_code ?? t('scriptRunner.noExit') }}</td>
              <td>{{ formatDateTime(run.started_at) }}</td>
              <td>{{ run.duration_ms !== null ? `${(run.duration_ms / 1000).toFixed(2)}s` : '—' }}</td>
              <td>
                <div class="btn-group">
                  <button class="macos-button" @click="viewLogs(run)">
                    <Icons name="terminal" size="11" />
                    <span>{{ t('scriptRunner.viewLogs') }}</span>
                  </button>
                  <button
                    v-if="run.status === 'running'"
                    class="macos-button danger"
                    @click="killRun(run)"
                  >
                    <Icons name="stop" size="11" />
                    <span>{{ t('scriptRunner.kill') }}</span>
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div v-if="selectedRun" class="section-card log-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('scriptRunner.logTitle') }} · {{ selectedRun.script_name }}</span>
          <span class="run-status" :class="selectedRun.status">{{ statusLabel(selectedRun.status) }}</span>
        </div>
        <div class="toolbar-actions">
          <span class="muted">
            {{ selectedRun.status === 'running' ? t('scriptRunner.logLive') : t('scriptRunner.logFinished') }}
          </span>
          <button class="macos-button" @click="closeLogs">{{ t('scriptRunner.close') }}</button>
        </div>
      </div>
      <p v-if="selectedRun.error_message" class="script-error">{{ selectedRun.error_message }}</p>
      <div class="log-grid">
        <div class="log-pane">
          <div class="log-label">{{ t('scriptRunner.stdout') }}</div>
          <pre class="log-output">{{ stdoutText || t('scriptRunner.logEmpty') }}</pre>
        </div>
        <div class="log-pane">
          <div class="log-label">{{ t('scriptRunner.stderr') }}</div>
          <pre class="log-output stderr">{{ stderrText || t('scriptRunner.logEmpty') }}</pre>
        </div>
      </div>
    </div>

    <div v-if="pendingDelete" class="modal-overlay" @click.self="pendingDelete = null">
      <div class="modal-card confirm-card">
        <span class="card-title">{{ t('scriptRunner.deleteConfirm', { name: pendingDelete.name }) }}</span>
        <div class="modal-actions">
          <button class="macos-button" @click="pendingDelete = null">{{ t('scriptRunner.cancel') }}</button>
          <button class="macos-button danger" @click="confirmDelete">{{ t('scriptRunner.delete') }}</button>
        </div>
      </div>
    </div>

    <div v-if="formOpen" class="modal-overlay" @click.self="closeForm">
      <div class="modal-card">
        <div class="modal-header">
          <span class="card-title">
            {{ form.id ? t('scriptRunner.formEditTitle') : t('scriptRunner.formNewTitle') }}
          </span>
          <button class="macos-button" @click="closeForm">
            <Icons name="close" size="12" />
          </button>
        </div>
        <div class="script-form">
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldName') }}</span>
            <input v-model="form.name" class="macos-input" type="text" maxlength="200" />
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldType') }}</span>
            <select v-model="form.script_type" class="macos-input">
              <option v-for="kind in ALL_TYPES" :key="kind" :value="kind">
                {{ typeLabel(kind) }}
              </option>
            </select>
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldMode') }}</span>
            <select v-model="form.is_inline" class="macos-input">
              <option :value="true">{{ t('scriptRunner.inline') }}</option>
              <option :value="false">{{ t('scriptRunner.file') }}</option>
            </select>
          </label>
          <label class="form-field form-wide">
            <span>{{ form.is_inline ? t('scriptRunner.fieldInline') : t('scriptRunner.fieldFile') }}</span>
            <textarea
              v-if="form.is_inline"
              v-model="form.content_or_path"
              class="macos-input script-textarea"
              spellcheck="false"
            ></textarea>
            <input
              v-else
              v-model="form.content_or_path"
              class="macos-input"
              type="text"
              :placeholder="t('scriptRunner.fieldFilePlaceholder')"
            />
            <small class="form-hint">{{ t('scriptRunner.fieldInlineHint') }}</small>
          </label>
          <label class="form-field form-wide">
            <span>{{ t('scriptRunner.fieldArgs') }}</span>
            <textarea
              v-model="form.argsText"
              class="macos-input args-textarea"
              spellcheck="false"
            ></textarea>
            <small class="form-hint">{{ t('scriptRunner.fieldArgsHint') }}</small>
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldWorkingDir') }}</span>
            <input
              v-model="form.working_dir"
              class="macos-input"
              type="text"
              :placeholder="t('scriptRunner.fieldWorkingDirPlaceholder')"
            />
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldHotkey') }}</span>
            <input
              v-model="form.hotkey"
              class="macos-input"
              type="text"
              placeholder="PrimaryMod+Alt+F9"
            />
            <small class="form-hint">{{ t('scriptRunner.fieldHotkeyHint') }}</small>
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldTimeout') }}</span>
            <input v-model.number="form.timeout_secs" class="macos-input" type="number" min="1" max="86400" />
          </label>
          <label class="form-field">
            <span>{{ t('scriptRunner.fieldOutput') }}</span>
            <select v-model="form.output_handling" class="macos-input">
              <option value="stream">{{ t('scriptRunner.outputStream') }}</option>
              <option value="final">{{ t('scriptRunner.outputFinal') }}</option>
              <option value="discard">{{ t('scriptRunner.outputDiscard') }}</option>
            </select>
          </label>
          <label class="form-field checkbox-field form-wide">
            <input v-model="form.show_in_tray" type="checkbox" />
            <span>{{ t('scriptRunner.fieldTray') }}</span>
          </label>
          <p v-if="formError" class="script-error form-wide">{{ formError }}</p>
        </div>
        <div class="modal-actions">
          <button class="macos-button" @click="closeForm">{{ t('scriptRunner.cancel') }}</button>
          <button class="macos-button primary" :disabled="saving" @click="saveScript">
            {{ saving ? t('scriptRunner.saving') : t('scriptRunner.save') }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import {
  invokeIpc,
  type ScriptEnvCapabilities,
  type ScriptInterpreter,
  type ScriptKind,
  type ScriptLogEntry,
  type ScriptRecord,
  type ScriptRunRecord,
  type ScriptRunStatus,
} from '../api/ipc';
import { t } from '../utils/i18n';
import Icons from './Icons.vue';

const ALL_TYPES: ScriptKind[] = [
  'python',
  'node',
  'shell',
  'applescript',
  'powershell',
  'batch',
  'vbscript',
];

const interpreters = ref<ScriptInterpreter[]>([]);
const environment = ref<ScriptEnvCapabilities | null>(null);
const scripts = ref<ScriptRecord[]>([]);
const runs = ref<ScriptRunRecord[]>([]);
const error = ref('');
const formError = ref('');
const saving = ref(false);
const formOpen = ref(false);
const pendingDelete = ref<ScriptRecord | null>(null);
const selectedRun = ref<ScriptRunRecord | null>(null);
const logEntries = ref<ScriptLogEntry[]>([]);
let lastSeq = 0;
let refreshTimer: ReturnType<typeof setInterval> | undefined;
let logTimer: ReturnType<typeof setInterval> | undefined;

interface ScriptForm {
  id: number | null;
  name: string;
  script_type: ScriptKind;
  is_inline: boolean;
  content_or_path: string;
  argsText: string;
  working_dir: string;
  hotkey: string;
  show_in_tray: boolean;
  timeout_secs: number;
  output_handling: string;
}

const form = reactive<ScriptForm>(emptyForm());

function emptyForm(): ScriptForm {
  return {
    id: null,
    name: '',
    script_type: 'shell',
    is_inline: true,
    content_or_path: '',
    argsText: '',
    working_dir: '',
    hotkey: '',
    show_in_tray: false,
    timeout_secs: 60,
    output_handling: 'stream',
  };
}

function typeLabel(type: ScriptKind): string {
  const key = `scriptRunner.types.${type}`;
  const translated = t(key);
  return translated !== key ? translated : type;
}

function statusLabel(status: ScriptRunStatus): string {
  return t(`scriptRunner.status.${status}`);
}

function previewText(value: string): string {
  const singleLine = value.replace(/\s+/g, ' ').trim();
  return singleLine.length > 64 ? `${singleLine.slice(0, 64)}…` : singleLine;
}

function formatDateTime(timestamp: number): string {
  const date = new Date(timestamp);
  const pad = (number: number) => String(number).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

function reportError(err: unknown) {
  const message = err instanceof Error ? err.message : String(err);
  error.value = t('scriptRunner.operationFailed', { message });
}

async function refreshProbes() {
  const result = await invokeIpc<{
    interpreters: ScriptInterpreter[];
    environment: ScriptEnvCapabilities;
  }>('script_probe_interpreters');
  interpreters.value = result.interpreters;
  environment.value = result.environment;
}

async function refreshScripts() {
  scripts.value = await invokeIpc<ScriptRecord[]>('script_list');
}

async function refreshRuns() {
  runs.value = await invokeIpc<ScriptRunRecord[]>('script_list_runs', { limit: 50 });
}

async function refreshAll(withProbe = false) {
  error.value = '';
  try {
    if (withProbe || interpreters.value.length === 0) {
      await refreshProbes();
    }
    await Promise.all([refreshScripts(), refreshRuns()]);
    if (selectedRun.value) {
      const latest = runs.value.find((run) => run.run_id === selectedRun.value?.run_id);
      if (latest) selectedRun.value = latest;
    }
  } catch (err) {
    reportError(err);
  }
}

function openCreate() {
  Object.assign(form, emptyForm());
  formError.value = '';
  formOpen.value = true;
}

function openEdit(script: ScriptRecord) {
  Object.assign(form, {
    id: script.id,
    name: script.name,
    script_type: script.script_type,
    is_inline: script.is_inline,
    content_or_path: script.content_or_path,
    argsText: script.args.join('\n'),
    working_dir: script.working_dir || '',
    hotkey: script.hotkey || '',
    show_in_tray: script.show_in_tray,
    timeout_secs: script.timeout_secs,
    output_handling: script.output_handling,
  });
  formError.value = '';
  formOpen.value = true;
}

function closeForm() {
  formOpen.value = false;
}

function parseArgs(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

async function saveScript() {
  if (!form.name.trim() || !form.content_or_path.trim()) {
    formError.value = t('scriptRunner.requiredFields');
    return;
  }
  saving.value = true;
  formError.value = '';
  try {
    await invokeIpc('script_save', {
      script: {
        id: form.id,
        name: form.name.trim(),
        script_type: form.script_type,
        is_inline: form.is_inline,
        content_or_path: form.content_or_path,
        args: parseArgs(form.argsText),
        working_dir: form.working_dir.trim() || null,
        hotkey: form.hotkey.trim() || null,
        show_in_tray: form.show_in_tray,
        timeout_secs: Number(form.timeout_secs) || 60,
        output_handling: form.output_handling,
      },
    });
    formOpen.value = false;
    await refreshAll(true);
  } catch (err) {
    formError.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}

function deleteScript(script: ScriptRecord) {
  pendingDelete.value = script;
}

async function confirmDelete() {
  const script = pendingDelete.value;
  if (!script) return;
  pendingDelete.value = null;
  error.value = '';
  try {
    await invokeIpc('script_delete', { script_id: script.id });
    if (selectedRun.value?.script_id === script.id) closeLogs();
    await refreshAll(true);
  } catch (err) {
    reportError(err);
  }
}

async function runScript(script: ScriptRecord) {
  error.value = '';
  try {
    const run = await invokeIpc<ScriptRunRecord>('script_execute', {
      script_id: script.id,
      args: [],
    });
    selectedRun.value = run;
    logEntries.value = [];
    lastSeq = 0;
    await refreshRuns();
    startLogPolling();
  } catch (err) {
    reportError(err);
  }
}

async function killRun(run: ScriptRunRecord) {
  error.value = '';
  try {
    await invokeIpc('script_kill', { run_id: run.run_id });
    await refreshRuns();
  } catch (err) {
    reportError(err);
  }
}

async function viewLogs(run: ScriptRunRecord) {
  selectedRun.value = run;
  logEntries.value = [];
  lastSeq = 0;
  await pollLogs();
  if (selectedRun.value?.status === 'running') startLogPolling();
}

function closeLogs() {
  stopLogPolling();
  selectedRun.value = null;
  logEntries.value = [];
  lastSeq = 0;
}

function startLogPolling() {
  stopLogPolling();
  logTimer = setInterval(pollLogs, 350);
}

function stopLogPolling() {
  if (logTimer) {
    clearInterval(logTimer);
    logTimer = undefined;
  }
}

async function pollLogs() {
  const current = selectedRun.value;
  if (!current) return;
  try {
    const result = await invokeIpc<{
      entries: ScriptLogEntry[];
      run: ScriptRunRecord | null;
      live: boolean;
    }>('script_get_logs', { run_id: current.run_id, since_seq: lastSeq });
    if (result.entries.length > 0) {
      logEntries.value.push(...result.entries);
      lastSeq = result.entries[result.entries.length - 1].seq;
    }
    if (result.run) selectedRun.value = result.run;
    if (!result.live) {
      stopLogPolling();
      await refreshRuns();
    }
  } catch (err) {
    stopLogPolling();
    reportError(err);
  }
}

function streamText(stream: 'stdout' | 'stderr'): string {
  const run = selectedRun.value;
  if (!run) return '';
  if (run.status !== 'running') {
    return stream === 'stdout' ? run.stdout_tail : run.stderr_tail;
  }
  return logEntries.value
    .filter((entry) => entry.stream === stream)
    .map((entry) => entry.text)
    .join('');
}

const stdoutText = computed(() => streamText('stdout'));
const stderrText = computed(() => streamText('stderr'));

onMounted(async () => {
  await refreshAll(true);
  refreshTimer = setInterval(() => refreshAll(false), 2000);
});

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer);
  stopLogPolling();
});
</script>

<style scoped>
.script-runner {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.card-header-bar {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}

.card-title {
  font-size: 13px;
  font-weight: 600;
  display: block;
}

.card-desc {
  font-size: 11px;
  color: var(--macos-text-tertiary);
}

.toolbar-actions,
.btn-group {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.script-error {
  color: var(--macos-danger);
  background: var(--macos-danger-subtle);
  border: 1px solid rgba(255, 69, 58, 0.25);
  border-radius: var(--macos-radius-sm);
  padding: 7px 10px;
  font-size: 12px;
  user-select: text;
}

.probe-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
  gap: 8px;
}

.probe-item {
  display: flex;
  align-items: center;
  gap: 9px;
  background: rgba(0, 0, 0, 0.16);
  border: 1px solid var(--macos-border-subtle);
  border-radius: 6px;
  padding: 8px 10px;
  min-width: 0;
}

.probe-item.unavailable {
  opacity: 0.72;
}

.probe-status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6e6e73;
  flex: none;
}

.probe-status-dot.ready {
  background: var(--macos-success);
  box-shadow: 0 0 6px rgba(48, 209, 88, 0.4);
}

.probe-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.probe-name {
  font-weight: 600;
  font-size: 12px;
}

.probe-path {
  font-size: 10px;
  color: var(--macos-text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.probe-badge {
  font-size: 10px;
  color: var(--macos-text-secondary);
  flex: none;
}

.env-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  font-size: 11px;
  color: var(--macos-text-secondary);
}

.env-title {
  font-weight: 600;
}

.env-pill {
  border: 1px solid var(--macos-border-subtle);
  border-radius: 999px;
  padding: 2px 8px;
  color: var(--macos-text-tertiary);
}

.env-pill.ready {
  color: var(--macos-success);
  border-color: rgba(48, 209, 88, 0.35);
  background: var(--macos-success-subtle);
}

.env-detail {
  color: var(--macos-text-tertiary);
}

.table-container {
  overflow-x: auto;
}

.actions-col {
  min-width: 210px;
}

.type-pill,
.source-pill {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.08);
  color: var(--macos-text-secondary);
}

.source-pill.file {
  color: var(--macos-accent);
}

.source-preview {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  color: var(--macos-text-tertiary);
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mono-cell {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hotkey-pill {
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 4px;
  padding: 2px 6px;
  font-family: inherit;
  font-size: 11px;
}

.muted {
  color: var(--macos-text-tertiary);
  font-size: 11px;
}

.inline-warning {
  font-size: 10px;
  color: var(--macos-danger);
  max-width: 160px;
}

.run-id {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 9px;
  color: var(--macos-text-tertiary);
}

.run-status {
  display: inline-block;
  font-size: 10px;
  border-radius: 999px;
  padding: 2px 8px;
  background: rgba(255, 255, 255, 0.07);
  color: var(--macos-text-secondary);
}

.run-status.running {
  color: var(--macos-accent);
  background: var(--macos-accent-subtle);
}

.run-status.succeeded {
  color: var(--macos-success);
  background: var(--macos-success-subtle);
}

.run-status.failed,
.run-status.spawn_failed,
.run-status.timed_out,
.run-status.cancelled,
.run-status.interrupted {
  color: var(--macos-danger);
  background: var(--macos-danger-subtle);
}

.log-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

@media (max-width: 720px) {
  .log-grid {
    grid-template-columns: 1fr;
  }
}

.log-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.log-label {
  font-size: 10px;
  color: var(--macos-text-tertiary);
  margin-bottom: 4px;
}

.log-output {
  margin: 0;
  min-height: 150px;
  max-height: 300px;
  overflow: auto;
  background: rgba(0, 0, 0, 0.28);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-sm);
  padding: 8px 10px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  line-height: 1.45;
  color: var(--macos-text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
}

.log-output.stderr {
  color: var(--macos-danger);
}

.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.36);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
  padding: 24px;
}

.modal-card {
  width: min(760px, 100%);
  max-height: 88vh;
  overflow-y: auto;
  background: var(--macos-bg-card-active);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-lg);
  box-shadow: var(--macos-shadow-window);
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.script-form {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px 12px;
}

.form-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11px;
  color: var(--macos-text-secondary);
}

.form-field.form-wide {
  grid-column: 1 / -1;
}

.checkbox-field {
  flex-direction: row;
  align-items: center;
}

.macos-input {
  width: 100%;
  background: var(--macos-bg-input);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-sm);
  padding: 7px 9px;
  color: var(--macos-text-primary);
  font: inherit;
  font-size: 12px;
  outline: none;
  user-select: text;
}

.macos-input:focus {
  border-color: var(--macos-accent);
  box-shadow: 0 0 0 2px var(--macos-accent-subtle);
}

.script-textarea {
  min-height: 150px;
  resize: vertical;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.args-textarea {
  min-height: 70px;
  resize: vertical;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.form-hint {
  color: var(--macos-text-tertiary);
  font-size: 10px;
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.confirm-card {
  width: min(440px, 100%);
  gap: 18px;
}
</style>
