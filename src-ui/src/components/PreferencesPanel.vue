<template>
  <div class="preferences">
    <section class="preference-section">
      <h2>{{ t('desktop.appearance') }}</h2>
      <div class="preference-row"><label for="display-language">{{ t('desktop.language') }}</label><select id="display-language" :value="currentLocale" @change="changeLanguage(($event.target as HTMLSelectElement).value)"><option v-for="locale in LOCALE_OPTIONS" :key="locale.code" :value="locale.code">{{ locale.nativeName }}</option></select></div>
      <div class="preference-row"><label for="display-theme">{{ t('desktop.theme') }}</label><select id="display-theme" :value="currentThemeMode" @change="changeTheme(($event.target as HTMLSelectElement).value)"><option value="system">{{ t('desktop.system') }}</option><option value="light">{{ t('desktop.light') }}</option><option value="dark">{{ t('desktop.dark') }}</option></select></div>
      <div class="preference-row"><span>{{ t('desktop.accent') }}</span><div class="accent-options" role="group" :aria-label="t('desktop.accent')"><button v-for="color in ACCENT_PALETTE" :key="color.id" :style="{ background: isDarkActive ? color.darkHex : color.lightHex }" :aria-label="t(`theme.colors.${color.id}`)" :aria-pressed="currentAccent === color.id" :class="{ selected: currentAccent === color.id }" @click="setAccentColor(color.id)"><span v-if="currentAccent === color.id" aria-hidden="true">✓</span></button></div></div>
    </section>
    <section class="preference-section shortcut-section">
      <div class="section-heading"><div><h2>{{ t('desktop.shortcuts') }}</h2><p>{{ t('desktop.shortcutsHint') }}</p></div><button class="check-all" :disabled="loading || checkingAll || saving !== null" @click="checkAllShortcuts">{{ checkingAll ? t('desktop.checkingShortcuts') : t('desktop.checkShortcuts') }}</button></div>
      <p v-if="scanSummary" class="scan-summary" :class="{ 'has-issues': scanSummary.issues > 0 }" role="status">{{ t('desktop.checkSummary', scanSummary) }}</p>
      <p v-if="loading" class="muted" role="status">{{ t('desktop.loading') }}</p>
      <details v-for="group in shortcutGroups" :key="group.module" class="shortcut-group">
        <summary><h3>{{ moduleName(group.module) }} <span v-if="!group.enabled">{{ t('desktop.moduleOff') }}</span></h3><span v-if="groupIssues(group.rows)" class="group-issues">{{ t('desktop.checkIssueCount', { count: groupIssues(group.rows) }) }}</span></summary>
        <div v-for="shortcut in group.rows" :key="shortcut.spec.action_id" class="shortcut-row">
          <label :for="`hotkey-${shortcut.spec.action_id}`">{{ actionName(shortcut) }}</label>
          <div class="shortcut-controls">
            <input :id="`hotkey-${shortcut.spec.action_id}`" class="shortcut-input" :class="{ recording: recording === shortcut.spec.action_id }" type="text" readonly :value="recording === shortcut.spec.action_id ? t('desktop.pressKeys') : draft[shortcut.spec.action_id] ? displayRecordedShortcut(draft[shortcut.spec.action_id], status?.target_os || '') : shortcut.display_label || t('desktop.notAssigned')" :aria-label="t('desktop.recordShortcut', { name: actionName(shortcut) })" :disabled="saving !== null || shortcut.editable === false" @focus="setRecording(shortcut.spec.action_id)" @blur="setRecording(null)" @keydown="recordShortcut($event, shortcut)" />
            <button class="save-shortcut" :disabled="saving !== null || checkingAll || checking[shortcut.spec.action_id] || isBlocked(checks[shortcut.spec.action_id]) || !draft[shortcut.spec.action_id]" @click="saveShortcut(shortcut, draft[shortcut.spec.action_id])">{{ saving === shortcut.spec.action_id ? t('desktop.saving') : t('desktop.save') }}</button>
            <button class="reset-shortcut" :disabled="saving !== null || shortcut.editable === false" :aria-label="t('desktop.resetShortcut', { name: actionName(shortcut) })" @click="saveShortcut(shortcut, null)">{{ shortcut.optional ? t('desktop.clear') : t('desktop.reset') }}</button>
          </div>
          <p v-if="checking[shortcut.spec.action_id]" class="check-result" role="status">{{ t('desktop.checkingShortcuts') }}</p>
          <p v-else-if="checks[shortcut.spec.action_id] && checks[shortcut.spec.action_id].status !== 'unassigned'" class="check-result" :class="{ blocked: isBlocked(checks[shortcut.spec.action_id]), available: checks[shortcut.spec.action_id].status === 'available' }" :title="checks[shortcut.spec.action_id].detail || ''" role="status">{{ checkMessage(checks[shortcut.spec.action_id]) }}</p>
          <p v-else-if="shortcut.native_error" class="row-error" :title="shortcut.native_error">{{ t('desktop.shortcutUnavailable') }}</p>
        </div>
      </details>
    </section>
    <p v-if="notice" class="preference-notice" :class="{ error: failed }" :role="failed ? 'alert' : 'status'">{{ notice }}</p>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { invokeIpc, type KernelStatus, type ShortcutPreference, type ShortcutCheck } from '../api/ipc';
import { t, currentLocale, LOCALE_OPTIONS, setLocale, type SupportedLocale } from '../utils/i18n';
import { currentThemeMode, currentAccent, isDarkActive, ACCENT_PALETTE, setThemeMode, setAccentColor, type ThemeMode } from '../utils/theme';
import { FEATURES, shortcutFromEvent, displayRecordedShortcut } from '../utils/features';
defineProps<{ status: KernelStatus | null }>();
const emit = defineEmits<{ changed: [] }>();
const shortcuts = ref<ShortcutPreference[]>([]);
const draft = ref<Record<string, string>>({});
const recording = ref<string | null>(null);
const saving = ref<string | null>(null);
const loading = ref(true);
const notice = ref('');
const failed = ref(false);
const checks = ref<Record<string, ShortcutCheck>>({});
const checking = ref<Record<string, boolean>>({});
const checkingAll = ref(false);
const scanSummary = ref<{ count: number; issues: number } | null>(null);
const generations: Record<string, number> = {};
let changeEpoch = 0;
function isBlocked(check?: ShortcutCheck) { return !!check && ['internal_conflict', 'os_unavailable', 'invalid', 'probe_error'].includes(check.status); }
function groupIssues(rows: ShortcutPreference[]) { return rows.filter(row => isBlocked(checks.value[row.spec.action_id])).length; }
function checkMessage(check: ShortcutCheck): string {
  if (check.status === 'internal_conflict') {
    const names = (check.conflicts || []).map(row => `${moduleName(row.spec.module_id)} · ${actionName(row)}${row.enabled ? '' : ` (${t('desktop.moduleOff')})`}`).join(' / ');
    return t('desktop.checkInternal', { names });
  }
  return t(`desktop.checkStatus.${check.status}`);
}
async function checkShortcut(actionId: string, hotkey: string | null): Promise<ShortcutCheck> {
  const token = generations[actionId] = (generations[actionId] || 0) + 1;
  const epoch = changeEpoch;
  checking.value[actionId] = true;
  let result: ShortcutCheck;
  try {
    result = await invokeIpc<ShortcutCheck>('kernel_check_shortcut', { action_id: actionId, hotkey });
    if (!result || typeof result.status !== 'string') throw new Error('Invalid shortcut check response');
  } catch (err) { result = { action_id: actionId, hotkey: hotkey || '', status: 'probe_error', detail: String(err) }; }
  if (!disposed && generations[actionId] === token) {
    checking.value[actionId] = false;
    if (changeEpoch === epoch) checks.value[actionId] = result;
  }
  return result;
}
async function checkAllShortcuts() {
  const epoch = ++changeEpoch;
  checkingAll.value = true; scanSummary.value = null;
  try {
    const result = await invokeIpc<{ checks: ShortcutCheck[] }>('kernel_check_shortcuts');
    const merged: Record<string, ShortcutCheck> = Object.fromEntries(result.checks.map(check => [check.action_id, check]));
    // Include unsaved candidates, rather than reporting an old configured key as available.
    for (const [id, hotkey] of Object.entries(draft.value)) {
      merged[id] = await invokeIpc<ShortcutCheck>('kernel_check_shortcut', { action_id: id, hotkey });
    }
    if (!disposed && changeEpoch === epoch) {
      checks.value = merged;
      scanSummary.value = { count: Object.values(merged).filter(check => check.status !== 'unassigned').length, issues: Object.values(merged).filter(isBlocked).length };
    }
  } catch (err) { if (!disposed && changeEpoch === epoch) showError(err); }
  finally { checkingAll.value = false; }
}
let recordingQueue: Promise<unknown> = Promise.resolve();
function setRecording(id: string | null) {
  recording.value = id;
  recordingQueue = recordingQueue.catch(() => {}).then(() => invokeIpc('kernel_set_shortcut_recording', { recording: id !== null })).catch(showError);
}
const shortcutGroups = computed(() => {
  const groups = new Map<string, { module: string; enabled: boolean; rows: ShortcutPreference[] }>();
  for (const row of shortcuts.value) {
    let group = groups.get(row.spec.module_id);
    if (!group) { group = { module: row.spec.module_id, enabled: row.enabled, rows: [] }; groups.set(row.spec.module_id, group); }
    group.rows.push(row);
  }
  return [...groups.values()].sort((a, b) => FEATURES.findIndex(f => f.module === a.module) - FEATURES.findIndex(f => f.module === b.module));
});
function moduleName(id: string) { const key = `moduleManager.names.${id}`; return t(key) === key ? id : t(key); }
function actionName(shortcut: ShortcutPreference) { if (shortcut.spec.action_id.endsWith(':open_panel')) return t('desktop.openPanel'); const key = `desktop.actions.${shortcut.spec.action_id.replace(':', '_')}`; return t(key) === key ? shortcut.spec.description : t(key); }
function showError(err: unknown) {
  const message = err instanceof Error ? err.message : String(err);
  notice.value = message.includes('Shortcut conflict detected') ? t('desktop.shortcutConflict') : message;
  failed.value = true;
}
function acceptRecordedShortcut(hotkey: string) {
  const id = recording.value;
  if (!id || !hotkey) return;
  ++changeEpoch; scanSummary.value = null; delete checks.value[id];
  draft.value[id] = hotkey;
  setRecording(null);
  if (typeof document !== 'undefined') (document.activeElement as HTMLElement | null)?.blur();
  notice.value = ''; failed.value = false;
  void checkShortcut(id, hotkey);
}
async function loadShortcuts() {
  try { shortcuts.value = (await invokeIpc<{ shortcuts: ShortcutPreference[] }>('kernel_get_shortcut_preferences')).shortcuts; }
  catch (err) { showError(err); }
  finally { loading.value = false; }
}
function recordShortcut(event: KeyboardEvent, shortcut: ShortcutPreference) {
  if (event.key === 'Tab') return;
  event.preventDefault(); event.stopPropagation();
  if (event.key === 'Escape') { (event.target as HTMLInputElement).blur(); return; }
  const hotkey = shortcutFromEvent(event);
  if (!hotkey) return;
  // A normal DOM key event and a registered OS shortcut share this commit path.
  recording.value = shortcut.spec.action_id;
  acceptRecordedShortcut(hotkey);
  (event.target as HTMLInputElement).blur();
  notice.value = ''; failed.value = false;
}
async function saveShortcut(shortcut: ShortcutPreference, hotkey: string | null) {
  saving.value = shortcut.spec.action_id; notice.value = ''; failed.value = false;
  ++changeEpoch; scanSummary.value = null;
  try {
    const check = await checkShortcut(shortcut.spec.action_id, hotkey);
    if (isBlocked(check)) { notice.value = checkMessage(check); failed.value = true; return; }
    const result = await invokeIpc<{ shortcuts: ShortcutPreference[] }>('kernel_update_shortcut', { action_id: shortcut.spec.action_id, hotkey });
    shortcuts.value = result.shortcuts;
    delete draft.value[shortcut.spec.action_id];
    notice.value = t('desktop.shortcutSaved');
    emit('changed');
  } catch (err) { showError(err); }
  finally { saving.value = null; }
}
async function changeLanguage(value: string) {
  try { await setLocale(value as SupportedLocale, true); emit('changed'); }
  catch (err) { showError(err); }
}
async function changeTheme(value: string) {
  try { await invokeIpc('kernel_update_general_config', { theme: value }); setThemeMode(value as ThemeMode); emit('changed'); }
  catch (err) { showError(err); }
}
let unlistenRecorded: (() => void) | undefined;
let disposed = false;
onMounted(async () => {
  const events = (window as any).__TAURI__?.event;
  try {
    if (typeof events?.listen === 'function') {
      const unlisten = await events.listen('shortcut://recorded', (event: { payload: { hotkey: string } }) => acceptRecordedShortcut(event.payload.hotkey));
      if (disposed) unlisten(); else unlistenRecorded = unlisten;
    }
  } catch (err) { showError(err); }
  // Subscribe before showing fields: Windows consumes already registered keys.
  if (!disposed) await loadShortcuts();
});
onUnmounted(() => { disposed = true; unlistenRecorded?.(); setRecording(null); });
</script>

<style scoped>
.preferences { display: flex; flex-direction: column; gap: 26px; }
.preference-section h2 { font-size: 13px; font-weight: 600; margin: 0 0 10px; }
.preference-row { display: flex; align-items: center; justify-content: space-between; min-height: 43px; gap: 18px; border-bottom: 1px solid var(--macos-border-subtle); font-size: 12px; }
select { min-width: 150px; background: var(--macos-bg-input); border: 1px solid var(--macos-border-subtle); color: var(--macos-text-primary); border-radius: 6px; padding: 5px 8px; font-size: 12px; }
.accent-options { display: flex; align-items: center; gap: 8px; padding: 4px 2px; }
.accent-options button { width: 20px; height: 20px; border: 0; border-radius: 50%; color: white; font-size: 12px; cursor: pointer; }
.accent-options button.selected { outline: 1px solid var(--macos-text-secondary); outline-offset: 3px; }
.section-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.check-all { flex: 0 0 auto; background: var(--macos-bg-input); border: 1px solid var(--macos-border-subtle); border-radius: 6px; padding: 6px 10px; color: var(--macos-accent); font-size: 12px; cursor: pointer; }
.scan-summary { color: var(--macos-text-secondary); font-size: 11px; margin: 9px 0; }
.scan-summary.has-issues, .group-issues { color: var(--macos-danger); }
.group-issues { font-size: 11px; margin: 0 10px 0 auto; }
.check-result { font-size: 11px; color: var(--macos-text-secondary); grid-column: 1 / -1; padding-bottom: 6px; overflow-wrap: anywhere; }
.check-result.blocked { color: var(--macos-danger); }
.check-result.available { color: var(--macos-success); }
.section-heading p, .muted { font-size: 11px; color: var(--macos-text-secondary); }
.shortcut-group { border-bottom: 1px solid var(--macos-border-subtle); margin-top: 0; }
.shortcut-group summary { display: flex; align-items: center; justify-content: space-between; min-height: 36px; cursor: pointer; list-style: none; }
.shortcut-group summary::-webkit-details-marker { display: none; }
.shortcut-group summary::after { content: "›"; color: var(--macos-text-tertiary); font-size: 18px; transition: transform .15s; }
.shortcut-group[open] summary::after { transform: rotate(90deg); }
.shortcut-group[open] { padding-bottom: 8px; }
.shortcut-group h3 { font-size: 12px; color: var(--macos-text-primary); font-weight: 500; margin: 0; }
.shortcut-group h3 span { margin-left: 5px; color: var(--macos-text-tertiary); }
.shortcut-row { display: grid; grid-template-columns: minmax(90px, 1fr) auto; align-items: center; min-height: 42px; gap: 4px 12px; border-bottom: 1px solid var(--macos-border-subtle); }
.shortcut-row label { font-size: 12px; }
.shortcut-controls { display: flex; align-items: center; gap: 7px; }
.shortcut-input { width: 155px; text-align: center; color: var(--macos-text-primary); background: var(--macos-bg-input); border: 1px solid var(--macos-border-subtle); padding: 5px 4px; border-radius: 6px; font-family: inherit; font-size: 11px; cursor: pointer; }
.shortcut-input.recording { border-color: var(--macos-accent); background: var(--macos-accent-subtle); color: var(--macos-accent); }
.save-shortcut, .reset-shortcut { border: 0; background: none; color: var(--macos-accent); padding: 4px 0; cursor: pointer; font-size: 11px; white-space: nowrap; }
.reset-shortcut { color: var(--macos-text-secondary); }
button:disabled, input:disabled { opacity: .4; cursor: default; }
.preference-notice { position: sticky; bottom: -14px; background: var(--macos-bg-window); padding: 10px 0; font-size: 12px; color: var(--macos-success); overflow-wrap: anywhere; }
.preference-notice.error, .row-error { color: var(--macos-danger); }
.row-error { font-size: 11px; grid-column: 1 / -1; margin: 2px 0 8px; overflow-wrap: anywhere; }
button:focus-visible, select:focus-visible { outline: 2px solid var(--macos-accent); outline-offset: 3px; }
@media (max-width: 580px) { .shortcut-row { grid-template-columns: 1fr; padding: 8px 0; } .shortcut-controls { justify-content: space-between; } .shortcut-input { flex: 1; } }
</style>
