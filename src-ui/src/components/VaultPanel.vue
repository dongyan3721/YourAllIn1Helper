<template>
  <div class="vault-panel">
    <header class="vault-header">
      <div class="vault-heading">
        <span class="vault-heading-icon"><Icons name="shield" size="18" /></span>
        <div>
          <h2>{{ vaultText('title') }}</h2>
          <p>{{ vaultText('m1') }}</p>
        </div>
      </div>
      <div class="vault-header-actions">
        <span class="status-pill" :class="{ open: status.initialized && !status.locked }">
          {{ status.initialized && !status.locked ? vaultText('unlock') : vaultText('lock') }}
        </span>
        <button v-if="status.initialized && !status.locked" class="macos-button" @click="lockNow">
          <Icons name="lock" size="12" />
          <span>{{ vaultText('lock') }}</span>
        </button>
      </div>
    </header>

    <p v-if="localizedError" class="vault-message error" role="alert">{{ localizedError }}</p>
    <p v-if="notice" class="vault-message notice">{{ notice }}</p>

    <section v-if="!status.initialized" class="vault-card vault-auth">
      <div class="vault-auth-intro">
        <span class="vault-auth-icon"><Icons name="shield" size="22" /></span>
        <div>
          <h3>{{ vaultText('initialize') }}</h3>
          <p>{{ vaultText('m1') }}</p>
        </div>
      </div>
      <div class="vault-form-grid">
        <label class="vault-field full">
          <span>{{ vaultText('master') }}</span>
          <input v-model="masterPassword" type="password" autocomplete="new-password" />
        </label>
        <label class="vault-field full">
          <span>{{ vaultText('confirm') }}</span>
          <input v-model="masterConfirmation" type="password" autocomplete="new-password" />
        </label>
      </div>
      <button class="macos-button primary vault-auth-submit" :disabled="busy" @click="initializeVault">
        <Icons name="check" size="12" />
        <span>{{ vaultText('initialize') }}</span>
      </button>
    </section>

    <section v-else-if="status.locked" class="vault-card vault-auth">
      <div class="vault-auth-intro">
        <span class="vault-auth-icon"><Icons name="lock" size="22" /></span>
        <div>
          <h3>{{ vaultText('unlock') }}</h3>
          <p>{{ vaultText('m1') }}</p>
        </div>
      </div>
      <label class="vault-field">
        <span>{{ vaultText('master') }}</span>
        <input v-model="masterPassword" type="password" autocomplete="current-password" @keydown.enter="unlockVault" />
      </label>
      <button class="macos-button primary vault-auth-submit" :disabled="busy || status.unlocking" @click="unlockVault">
        <Icons name="key" size="12" />
        <span>{{ vaultText('unlock') }}</span>
      </button>
      <div class="vault-biometric">
        <button class="macos-button" disabled :title="vaultText('biometricDisabled')">Touch ID / Windows Hello — M2</button>
        <small>{{ vaultText('biometricDisabled') }}</small>
      </div>
    </section>

    <template v-else>
      <section class="vault-toolbar">
        <div class="macos-search-box vault-search">
          <Icons name="search" size="13" />
          <input v-model="searchQuery" class="macos-search-input" :placeholder="vaultText('search')" @keydown.enter="refreshEntries" />
        </div>
        <div class="vault-toolbar-actions">
          <button class="macos-button" :disabled="busy" @click="refreshEntries">
            <Icons name="refresh" size="12" />
            <span>{{ vaultText('searchButton') }}</span>
          </button>
          <button class="macos-button" :disabled="busy" @click="generatePassword">
            <Icons name="key" size="12" />
            <span>{{ vaultText('generate') }}</span>
          </button>
          <button class="macos-button primary" :disabled="busy" @click="startNew">
            <Icons name="plus" size="12" />
            <span>{{ vaultText('newEntry') }}</span>
          </button>
        </div>
      </section>

      <section v-if="generatedPassword" class="vault-secret-bar">
        <Icons name="key" size="14" />
        <strong>{{ vaultText('generated') }}</strong>
        <code>{{ generatedPassword }}</code>
        <button class="macos-button" @click="useGenerated">{{ vaultText('useGenerated') }}</button>
      </section>

      <div class="vault-layout">
        <section class="vault-list-pane">
          <div class="vault-section-heading">
            <span>{{ vaultText('title') }}</span>
            <span class="macos-badge-pill">{{ entries.length }}</span>
          </div>
          <div class="vault-list">
            <button
              v-for="entry in entries"
              :key="entry.id"
              class="vault-entry"
              :class="{ active: form.id === entry.id }"
              @click="editEntry(entry.id)"
            >
              <span class="vault-entry-avatar">{{ (entry.title || '?').slice(0, 1).toUpperCase() }}</span>
              <span class="vault-entry-copy">
                <strong>{{ entry.title }}</strong>
                <span>{{ entry.username }}</span>
                <small>{{ entry.url }}</small>
              </span>
              <span class="vault-reveal-action" :title="vaultText('reveal')" @click.stop="revealPassword(entry.id)">
                <Icons :name="revealedId === entry.id && revealedPassword ? 'eye-off' : 'eye'" size="14" />
              </span>
            </button>
            <p v-if="!entries.length" class="vault-empty">{{ vaultText('noEntries') }}</p>
          </div>
        </section>

        <section v-if="editing" class="vault-editor-pane">
          <div class="vault-section-heading">
            <span>{{ form.title || vaultText('newEntry') }}</span>
            <button class="macos-button subtle" :disabled="busy || !form.id" @click="copyPassword">
              <Icons name="copy" size="12" />
              <span>{{ vaultText('copy') }}</span>
            </button>
          </div>
          <div class="vault-form-grid">
            <label class="vault-field"><span>{{ vaultText('titleField') }}</span><input v-model="form.title" /></label>
            <label class="vault-field"><span>{{ vaultText('url') }}</span><input v-model="form.url" /></label>
            <label class="vault-field"><span>{{ vaultText('category') }}</span><input v-model="form.category" /></label>
            <label class="vault-field"><span>{{ vaultText('username') }}</span><input v-model="form.username" /></label>
            <label class="vault-field"><span>{{ vaultText('password') }}</span><input v-model="form.password" type="password" autocomplete="new-password" /></label>
            <label class="vault-field"><span>{{ vaultText('totp') }}</span><input v-model="form.totp_secret" type="password" autocomplete="off" /></label>
            <label class="vault-field full"><span>{{ vaultText('notes') }}</span><textarea v-model="form.notes" rows="4"></textarea></label>
            <label class="vault-check full"><input v-model="form.archived" type="checkbox" /><span>{{ vaultText('archived') }}</span></label>
          </div>
          <div class="vault-actions">
            <button class="macos-button primary" :disabled="busy" @click="saveEntry">
              <Icons name="check" size="12" />
              <span>{{ vaultText('save') }}</span>
            </button>
            <button class="macos-button" @click="cancelEdit">{{ vaultText('cancel') }}</button>
            <button v-if="form.id" class="macos-button danger" :disabled="busy" @click="requestDelete">
              <Icons name="trash" size="12" />
              <span>{{ vaultText('delete') }}</span>
            </button>
          </div>
          <section v-if="deleteConfirmation" class="vault-delete-confirm" role="alertdialog" :aria-label="vaultText('deleteConfirm')">
            <p>{{ vaultText('deleteConfirm') }}</p>
            <div class="vault-actions">
              <button class="macos-button" :disabled="busy" @click="cancelDelete">{{ vaultText('cancel') }}</button>
              <button class="macos-button danger" :disabled="busy" @click="deleteEntry">{{ vaultText('confirmDelete') }}</button>
            </div>
          </section>
        </section>

        <section v-else-if="revealedPassword" class="vault-editor-pane vault-reveal">
          <div class="vault-section-heading"><span>{{ vaultText('reveal') }}</span></div>
          <code>{{ revealedPassword }}</code>
          <button class="macos-button" @click="clearReveal">
            <Icons name="eye-off" size="12" />
            <span>{{ vaultText('hide') }}</span>
          </button>
        </section>
      </div>

      <section class="vault-card vault-change">
        <div class="vault-change-heading">
          <div>
            <h3>{{ vaultText('changeMaster') }}</h3>
            <small>{{ vaultText('changeMasterHint') }}</small>
          </div>
          <Icons name="key" size="15" />
        </div>
        <div class="vault-change-grid">
          <input v-model="oldMasterPassword" type="password" :placeholder="vaultText('oldMaster')" autocomplete="current-password" />
          <input v-model="newMasterPassword" type="password" :placeholder="vaultText('newMaster')" autocomplete="new-password" />
          <input v-model="newMasterConfirmation" type="password" :placeholder="vaultText('confirm')" autocomplete="new-password" />
          <button class="macos-button" :disabled="busy" @click="changeMasterPassword">{{ vaultText('changeMaster') }}</button>
        </div>
      </section>
    </template>
    <p v-if="busy" class="vault-busy"><span class="vault-busy-dot"></span>{{ vaultText('busy') }}</p>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invokeIpc } from '../api/ipc';
import Icons from './Icons.vue';
import {
  emptyVaultEntry,
  VAULT_REVEAL_MS,
  type VaultEntryDraft,
  type VaultEntryPage,
  type VaultEntrySummary,
  type VaultMutationResult,
  type VaultStatus,
} from '../api/vault';
import { vaultErrorText, vaultText } from '../utils/vault/text';

const status = ref<VaultStatus>({
  initialized: false, locked: true, unlocking: false, epoch: 0,
  auto_lock_seconds: 180, biometric_available: false,
  secret_copy_available: false, capability_error: null,
});
const masterPassword = ref('');
const masterConfirmation = ref('');
const oldMasterPassword = ref('');
const newMasterPassword = ref('');
const newMasterConfirmation = ref('');
const searchQuery = ref('');
const entries = ref<VaultEntrySummary[]>([]);
const form = ref<VaultEntryDraft>(emptyVaultEntry());
const deleteConfirmation = ref<{ id: string; record_version: number } | null>(null);
const editing = computed(() => form.value.id !== '' || form.value.title !== '' || form.value.record_version === 0 && newEntryOpen.value);
const newEntryOpen = ref(false);
const revealedPassword = ref('');
const revealedId = ref('');
const generatedPassword = ref('');
const error = ref('');
const localizedError = computed(() => error.value ? vaultErrorText(error.value) : '');
const notice = ref('');
const busy = ref(true);
let disposed = false;
let operationEpoch = 0;
let revealTimer: ReturnType<typeof setTimeout> | undefined;
let unlistenLocked: (() => void) | undefined;

function clearReveal() {
  if (revealTimer) clearTimeout(revealTimer);
  revealTimer = undefined;
  revealedPassword.value = '';
  revealedId.value = '';
  generatedPassword.value = '';
}

function scheduleRevealClear() {
  const timer = setTimeout(() => {
    if (revealTimer === timer) clearReveal();
  }, VAULT_REVEAL_MS);
  revealTimer = timer;
}

function clearSensitiveState() {
  operationEpoch += 1;
  clearReveal();
  masterPassword.value = '';
  masterConfirmation.value = '';
  oldMasterPassword.value = '';
  newMasterPassword.value = '';
  newMasterConfirmation.value = '';
  searchQuery.value = '';
  entries.value = [];
  form.value = emptyVaultEntry();
  deleteConfirmation.value = null;
  newEntryOpen.value = false;
  status.value = { ...status.value, locked: true, unlocking: false };
  busy.value = false;
}

function reportError(reason: unknown) {
  const message = reason instanceof Error ? reason.message : String(reason);
  // Keep the raw diagnostic internally so the computed message follows locale changes.
  error.value = message;
  if (message.includes('locked') || message.includes('disabled')) {
    clearSensitiveState();
  }
}

function currentOperation() {
  operationEpoch += 1;
  error.value = '';
  notice.value = '';
  return operationEpoch;
}

function isCurrent(operation: number) {
  return !disposed && operation === operationEpoch && !status.value.locked;
}

async function loadStatus() {
  const operation = ++operationEpoch;
  try {
    const next = await invokeIpc<VaultStatus>('vault_get_status');
    if (disposed || operation !== operationEpoch) return;
    status.value = next;
    if (!next.locked) await refreshEntries();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

async function initializeVault() {
  if (busy.value) return;
  const operation = ++operationEpoch;
  busy.value = true;
  error.value = '';
  const password = masterPassword.value;
  const confirmation = masterConfirmation.value;
  masterPassword.value = '';
  masterConfirmation.value = '';
  try {
    const next = await invokeIpc<VaultStatus>('vault_initialize', { password, confirmation });
    if (disposed || operation !== operationEpoch) {
      clearSensitiveState();
      return;
    }
    status.value = next;
    clearSensitiveState();
    status.value = { ...status.value, initialized: true, locked: true };
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

async function unlockVault() {
  if (busy.value || status.value.unlocking) return;
  const operation = ++operationEpoch;
  busy.value = true;
  error.value = '';
  const password = masterPassword.value;
  masterPassword.value = '';
  try {
    const next = await invokeIpc<VaultStatus>('vault_unlock_master_password', { password });
    if (disposed || operation !== operationEpoch) return;
    status.value = next;
    busy.value = false;
    await refreshEntries();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

function lockBackend() {
  void invokeIpc<VaultStatus>('vault_lock').catch(() => undefined);
}

function lockNow() {
  clearSensitiveState();
  notice.value = vaultText('lockedNotice');
  lockBackend();
}

async function refreshEntries() {
  if (status.value.locked) return;
  const operation = currentOperation();
  busy.value = true;
  try {
    const page = await invokeIpc<VaultEntryPage>('vault_list_entries', {
      search: searchQuery.value || null, cursor: null, limit: 50,
    });
    if (!isCurrent(operation)) return;
    entries.value = page.entries;
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

function startNew() {
  cancelDelete();
  clearReveal();
  form.value = emptyVaultEntry();
  newEntryOpen.value = true;
}

function cancelEdit() {
  cancelDelete();
  clearReveal();
  form.value = emptyVaultEntry();
  newEntryOpen.value = false;
}

async function editEntry(id: string) {
  cancelDelete();
  const operation = currentOperation();
  busy.value = true;
  clearReveal();
  try {
    const entry = await invokeIpc<VaultEntryDraft>('vault_get_entry', { id });
    if (!isCurrent(operation)) return;
    form.value = {
      id: entry.id, record_version: entry.record_version, archived: entry.archived,
      title: entry.title, url: entry.url, category: entry.category,
      username: entry.username, password: entry.password,
      totp_secret: entry.totp_secret, notes: entry.notes,
    };
    newEntryOpen.value = false;
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

async function saveEntry() {
  cancelDelete();
  const operation = currentOperation();
  busy.value = true;
  const snapshot = { ...form.value };
  try {
    await invokeIpc<VaultMutationResult>('vault_upsert_entry', {
      id: snapshot.id || null,
      expected_version: snapshot.id ? snapshot.record_version : null,
      input: {
        title: snapshot.title, url: snapshot.url, category: snapshot.category,
        username: snapshot.username, password: snapshot.password,
        totp_secret: snapshot.totp_secret, notes: snapshot.notes,
        archived: snapshot.archived,
      },
    });
    if (!isCurrent(operation)) return;
    cancelEdit();
    busy.value = false;
    await refreshEntries();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

function requestDelete() {
  if (busy.value || !form.value.id) return;
  deleteConfirmation.value = { id: form.value.id, record_version: form.value.record_version };
}

function cancelDelete() {
  deleteConfirmation.value = null;
}

async function deleteEntry() {
  const confirmed = deleteConfirmation.value;
  if (busy.value) return;
  cancelDelete();
  if (!confirmed || confirmed.id !== form.value.id || confirmed.record_version !== form.value.record_version) return;
  const operation = currentOperation();
  busy.value = true;
  const id = confirmed.id;
  const expectedVersion = confirmed.record_version;
  try {
    await invokeIpc('vault_delete_entry', { id, expected_version: expectedVersion });
    if (!isCurrent(operation)) return;
    cancelEdit();
    busy.value = false;
    await refreshEntries();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

async function copyPassword() {
  if (!form.value.id) return;
  const operation = currentOperation();
  busy.value = true;
  try {
    await invokeIpc('vault_copy_password', { id: form.value.id });
    if (!isCurrent(operation)) return;
    notice.value = vaultText('copied');
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

async function revealPassword(id: string) {
  if (revealedId.value === id && revealedPassword.value) {
    clearReveal();
    return;
  }
  const operation = currentOperation();
  clearReveal();
  try {
    const secret = await invokeIpc<string>('vault_reveal_password', { id });
    if (!isCurrent(operation)) return;
    revealedPassword.value = secret;
    revealedId.value = id;
    scheduleRevealClear();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  }
}

async function generatePassword() {
  const operation = currentOperation();
  clearReveal();
  try {
    const generated = await invokeIpc<string>('vault_generate_password', {
      policy: { length: 20, uppercase: true, lowercase: true, digits: true, symbols: true },
    });
    if (!isCurrent(operation)) return;
    generatedPassword.value = generated;
    scheduleRevealClear();
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  }
}

function useGenerated() {
  // Opening a new editor clears transient secrets, so retain the value first.
  const password = generatedPassword.value;
  if (!editing.value) startNew();
  form.value.password = password;
  clearReveal();
}

async function changeMasterPassword() {
  if (newMasterPassword.value !== newMasterConfirmation.value) {
    error.value = 'invalid vault input: master password confirmation differs';
    return;
  }
  const operation = currentOperation();
  busy.value = true;
  const oldPassword = oldMasterPassword.value;
  const newPassword = newMasterPassword.value;
  oldMasterPassword.value = '';
  newMasterPassword.value = '';
  newMasterConfirmation.value = '';
  try {
    await invokeIpc<VaultStatus>('vault_change_master_password', {
      old_password: oldPassword, new_password: newPassword,
    });
    if (!disposed && operation === operationEpoch) {
      clearSensitiveState();
      notice.value = vaultText('lockedNotice');
    }
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  } finally {
    if (!disposed && operation === operationEpoch) busy.value = false;
  }
}

function handleVisibilityLoss() {
  if (document.hidden) {
    clearSensitiveState();
    lockBackend();
  }
}

function handlePageHide() {
  clearSensitiveState();
  lockBackend();
}

async function registerLockedListener() {
  const eventApi = (window as any).__TAURI__?.event;
  if (typeof eventApi?.listen !== 'function') return;
  const unlisten = await eventApi.listen('vault://locked', () => {
    if (!disposed) {
      clearSensitiveState();
      notice.value = vaultText('lockedNotice');
    }
  });
  if (disposed) unlisten();
  else unlistenLocked = unlisten;
}

onMounted(async () => {
  window.addEventListener('pagehide', handlePageHide);
  document.addEventListener('visibilitychange', handleVisibilityLoss);
  await loadStatus();
  if (!disposed) await registerLockedListener();
});

onUnmounted(() => {
  disposed = true;
  clearSensitiveState();
  window.removeEventListener('pagehide', handlePageHide);
  document.removeEventListener('visibilitychange', handleVisibilityLoss);
  unlistenLocked?.();
  lockBackend();
});
</script>

<style scoped>
.vault-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
  max-width: 1180px;
  margin: 0 auto;
  color: var(--macos-text-primary);
  font-size: 12px;
}

.vault-header,
.vault-heading,
.vault-header-actions,
.vault-toolbar,
.vault-toolbar-actions,
.vault-actions,
.vault-section-heading,
.vault-change-heading {
  display: flex;
  align-items: center;
}

.vault-header {
  min-height: 42px;
  justify-content: space-between;
  gap: 16px;
  padding: 0 2px;
}

.vault-heading { gap: 10px; min-width: 0; }
.vault-heading-icon,
.vault-auth-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--macos-accent);
  background: var(--macos-accent-subtle);
  border: 1px solid rgba(var(--macos-accent-rgb), 0.12);
}
.vault-heading-icon { width: 34px; height: 34px; border-radius: 9px; }
.vault-header h2 { margin: 0; font-size: 17px; font-weight: 600; letter-spacing: -0.02em; }
.vault-header p,
.vault-auth-intro p,
.vault-card small {
  margin: 2px 0 0;
  color: var(--macos-text-secondary);
  font-size: 11px;
}
.vault-header-actions { gap: 8px; }

.vault-card,
.vault-list-pane,
.vault-editor-pane {
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-lg);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.03);
  backdrop-filter: blur(18px) saturate(150%);
  -webkit-backdrop-filter: blur(18px) saturate(150%);
}
.vault-card { padding: 16px; }

.vault-auth {
  display: grid;
  gap: 16px;
  width: min(100%, 460px);
  margin: 14px auto;
  padding: 22px;
}
.vault-auth-intro { display: flex; align-items: center; gap: 12px; }
.vault-auth-icon { width: 42px; height: 42px; border-radius: 11px; }
.vault-auth-intro h3 { margin: 0; font-size: 15px; font-weight: 600; }
.vault-auth-submit { width: 100%; min-height: 30px; }
.vault-biometric { display: grid; justify-items: center; gap: 6px; }

.vault-toolbar {
  gap: 10px;
  padding: 10px 12px;
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-lg);
}
.vault-search { flex: 1; min-width: 180px; }
.vault-toolbar-actions { gap: 7px; flex-shrink: 0; }

.vault-panel input,
.vault-panel textarea {
  box-sizing: border-box;
  width: 100%;
  min-width: 0;
  padding: 7px 9px;
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-sm);
  outline: none;
  background: var(--macos-bg-input);
  color: var(--macos-text-primary);
  font: inherit;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, background 0.15s ease;
}
.vault-panel input { height: 30px; }
/* Preserve room for the absolutely positioned search icon (left 9px + 13px icon). */
.vault-panel .macos-search-input { padding: 7px 9px 7px 30px; }
.vault-panel textarea { min-height: 82px; resize: vertical; line-height: 1.5; }
.vault-panel input::placeholder,
.vault-panel textarea::placeholder { color: var(--macos-text-tertiary); }
.vault-panel input:focus,
.vault-panel textarea:focus {
  border-color: var(--macos-accent);
  box-shadow: 0 0 0 2px var(--macos-accent-subtle);
  background: var(--macos-bg-card-hover);
}
.vault-panel button:disabled { cursor: not-allowed; opacity: 0.5; }
.vault-panel .macos-button.subtle {
  padding: 3px 8px;
  background: transparent;
  box-shadow: none;
  color: var(--macos-text-secondary);
}

.vault-secret-bar {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 12px;
  color: var(--macos-accent);
  background: var(--macos-accent-subtle);
  border: 1px solid rgba(var(--macos-accent-rgb), 0.18);
  border-radius: var(--macos-radius-md);
}
.vault-secret-bar code { flex: 1; min-width: 0; overflow-wrap: anywhere; color: var(--macos-text-primary); }

.vault-layout {
  display: grid;
  grid-template-columns: minmax(230px, 0.85fr) minmax(360px, 1.5fr);
  gap: 12px;
  align-items: start;
}
.vault-list-pane,
.vault-editor-pane { min-width: 0; padding: 12px; }
.vault-section-heading {
  justify-content: space-between;
  gap: 10px;
  min-height: 28px;
  margin-bottom: 10px;
  padding: 0 2px;
  color: var(--macos-text-secondary);
  font-size: 11px;
  font-weight: 600;
}
.vault-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 430px;
  overflow-y: auto;
}
.vault-entry {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 8px;
  border: 1px solid transparent;
  border-radius: var(--macos-radius-md);
  background: transparent;
  color: var(--macos-text-primary);
  text-align: left;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
}
.vault-entry:hover { background: var(--macos-bg-card-hover); border-color: var(--macos-border-subtle); }
.vault-entry.active { background: var(--macos-accent-subtle); border-color: rgba(var(--macos-accent-rgb), 0.24); }
.vault-entry-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 29px;
  height: 29px;
  border-radius: 7px;
  background: rgba(128, 128, 128, 0.12);
  color: var(--macos-text-secondary);
  font-size: 12px;
  font-weight: 600;
}
.vault-entry.active .vault-entry-avatar { color: var(--macos-accent); background: var(--macos-bg-card-hover); }
.vault-entry-copy { display: grid; flex: 1; min-width: 0; gap: 1px; }
.vault-entry-copy strong { overflow: hidden; text-overflow: ellipsis; font-size: 12px; font-weight: 500; white-space: nowrap; }
.vault-entry-copy span,
.vault-entry-copy small {
  overflow: hidden;
  color: var(--macos-text-secondary);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.vault-entry-copy small { color: var(--macos-text-tertiary); }
.vault-reveal-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 26px;
  height: 26px;
  border-radius: 5px;
  color: var(--macos-text-tertiary);
}
.vault-reveal-action:hover { color: var(--macos-accent); background: var(--macos-accent-subtle); }
.vault-empty { padding: 34px 12px; color: var(--macos-text-tertiary); text-align: center; }

.vault-form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 11px 12px; }
.vault-field { display: grid; gap: 5px; min-width: 0; color: var(--macos-text-secondary); font-size: 11px; }
.vault-field.full,
.vault-check.full { grid-column: 1 / -1; }
.vault-check { display: flex; align-items: center; gap: 7px; color: var(--macos-text-secondary); font-size: 11px; }
.vault-check input { width: 14px; height: 14px; min-height: 0; margin: 0; accent-color: var(--macos-accent); }
.vault-actions { gap: 7px; margin-top: 14px; }
.vault-delete-confirm {
  margin-top: 12px;
  padding: 11px;
  background: var(--macos-danger-subtle);
  border: 1px solid rgba(255, 69, 58, 0.22);
  border-radius: var(--macos-radius-md);
}
.vault-delete-confirm p { margin: 0; color: var(--macos-danger); }
.vault-delete-confirm .vault-actions { margin-top: 9px; }
.vault-reveal { display: grid; gap: 12px; }
.vault-reveal code {
  display: block;
  padding: 16px;
  border-radius: var(--macos-radius-md);
  background: rgba(128, 128, 128, 0.10);
  overflow-wrap: anywhere;
  font-size: 14px;
}

.vault-change { display: grid; gap: 12px; padding: 13px 14px; }
.vault-change-heading { justify-content: space-between; color: var(--macos-text-secondary); }
.vault-change-heading h3 { margin: 0; color: var(--macos-text-primary); font-size: 12px; font-weight: 600; }
.vault-change-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)) auto; gap: 8px; }

.vault-message {
  margin: 0;
  padding: 9px 11px;
  border: 1px solid transparent;
  border-radius: var(--macos-radius-md);
  font-size: 11px;
}
.vault-message.error { color: var(--macos-danger); background: var(--macos-danger-subtle); border-color: rgba(255, 69, 58, 0.18); }
.vault-message.notice { color: var(--macos-success); background: var(--macos-success-subtle); border-color: rgba(52, 199, 89, 0.18); }
.vault-busy { display: flex; align-items: center; justify-content: center; gap: 6px; margin: 0; color: var(--macos-text-tertiary); font-size: 10px; }
.vault-busy-dot { width: 5px; height: 5px; border-radius: 50%; background: var(--macos-accent); }

@media (max-width: 860px) {
  .vault-layout { grid-template-columns: 1fr; }
  .vault-change-grid { grid-template-columns: 1fr 1fr; }
}
@media (max-width: 620px) {
  .vault-header { align-items: flex-start; }
  .vault-header-actions .status-pill { display: none; }
  .vault-toolbar { align-items: stretch; flex-direction: column; }
  .vault-toolbar-actions { flex-wrap: wrap; }
  .vault-form-grid,
  .vault-change-grid { grid-template-columns: 1fr; }
}
</style>
