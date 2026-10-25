<template>
  <div class="vault-panel">
    <header class="vault-header">
      <div>
        <h2>{{ vaultText('title') }}</h2>
        <p>{{ vaultText('m1') }}</p>
      </div>
      <button v-if="status.initialized && !status.locked" class="vault-button" @click="lockNow">
        {{ vaultText('lock') }}
      </button>
    </header>

    <p v-if="error" class="vault-error" role="alert">{{ error }}</p>
    <p v-if="notice" class="vault-notice">{{ notice }}</p>

    <section v-if="!status.initialized" class="vault-card vault-auth">
      <h3>{{ vaultText('initialize') }}</h3>
      <label>{{ vaultText('master') }}<input v-model="masterPassword" type="password" autocomplete="new-password" /></label>
      <label>{{ vaultText('confirm') }}<input v-model="masterConfirmation" type="password" autocomplete="new-password" /></label>
      <button class="vault-button primary" :disabled="busy" @click="initializeVault">{{ vaultText('initialize') }}</button>
    </section>

    <section v-else-if="status.locked" class="vault-card vault-auth">
      <h3>{{ vaultText('unlock') }}</h3>
      <label>{{ vaultText('master') }}<input v-model="masterPassword" type="password" autocomplete="current-password" @keydown.enter="unlockVault" /></label>
      <button class="vault-button primary" :disabled="busy || status.unlocking" @click="unlockVault">{{ vaultText('unlock') }}</button>
      <button class="vault-button" disabled :title="vaultText('biometricDisabled')">Touch ID / Windows Hello — M2</button>
      <small>{{ vaultText('biometricDisabled') }}</small>
    </section>

    <template v-else>
      <section class="vault-card vault-toolbar">
        <input v-model="searchQuery" :placeholder="vaultText('search')" @keydown.enter="refreshEntries" />
        <button class="vault-button" :disabled="busy" @click="refreshEntries">{{ vaultText('searchButton') }}</button>
        <button class="vault-button primary" :disabled="busy" @click="startNew">{{ vaultText('newEntry') }}</button>
        <button class="vault-button" :disabled="busy" @click="generatePassword">{{ vaultText('generate') }}</button>
      </section>

      <section v-if="generatedPassword" class="vault-card vault-generated">
        <strong>{{ vaultText('generated') }}</strong>
        <code>{{ generatedPassword }}</code>
        <button class="vault-button" @click="useGenerated">{{ vaultText('useGenerated') }}</button>
      </section>

      <div class="vault-layout">
        <section class="vault-card vault-list">
          <button v-for="entry in entries" :key="entry.id" class="vault-entry" @click="editEntry(entry.id)">
            <strong>{{ entry.title }}</strong>
            <span>{{ entry.username }}</span>
            <small>{{ entry.url }}</small>
            <span class="vault-reveal-action" @click.stop="revealPassword(entry.id)">{{ vaultText('reveal') }}</span>
          </button>
          <p v-if="!entries.length">{{ vaultText('noEntries') }}</p>
        </section>

        <section v-if="editing" class="vault-card vault-editor">
          <label>{{ vaultText('titleField') }}<input v-model="form.title" /></label>
          <label>{{ vaultText('url') }}<input v-model="form.url" /></label>
          <label>{{ vaultText('category') }}<input v-model="form.category" /></label>
          <label>{{ vaultText('username') }}<input v-model="form.username" /></label>
          <label>{{ vaultText('password') }}<input v-model="form.password" type="password" autocomplete="new-password" /></label>
          <label>{{ vaultText('totp') }}<input v-model="form.totp_secret" type="password" autocomplete="off" /></label>
          <label>{{ vaultText('notes') }}<textarea v-model="form.notes" rows="4"></textarea></label>
          <label class="vault-check"><input v-model="form.archived" type="checkbox" />{{ vaultText('archived') }}</label>
          <div class="vault-actions">
            <button class="vault-button primary" :disabled="busy" @click="saveEntry">{{ vaultText('save') }}</button>
            <button class="vault-button" @click="cancelEdit">{{ vaultText('cancel') }}</button>
            <button v-if="form.id" class="vault-button danger" :disabled="busy" @click="deleteEntry">{{ vaultText('delete') }}</button>
          </div>
          <button class="vault-button" disabled :title="vaultText('copyDisabled')">{{ vaultText('copyDisabled') }}</button>
        </section>

        <section v-else-if="revealedPassword" class="vault-card vault-reveal">
          <code>{{ revealedPassword }}</code>
          <button class="vault-button" @click="clearReveal">{{ vaultText('hide') }}</button>
        </section>
      </div>

      <section class="vault-card vault-change">
        <h3>{{ vaultText('changeMaster') }}</h3>
        <small>{{ vaultText('changeMasterHint') }}</small>
        <div class="vault-change-grid">
          <input v-model="oldMasterPassword" type="password" :placeholder="vaultText('oldMaster')" autocomplete="current-password" />
          <input v-model="newMasterPassword" type="password" :placeholder="vaultText('newMaster')" autocomplete="new-password" />
          <input v-model="newMasterConfirmation" type="password" :placeholder="vaultText('confirm')" autocomplete="new-password" />
          <button class="vault-button" :disabled="busy" @click="changeMasterPassword">{{ vaultText('changeMaster') }}</button>
        </div>
      </section>
    </template>
    <p v-if="busy" class="vault-busy">{{ vaultText('busy') }}</p>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invokeIpc } from '../api/ipc';
import {
  emptyVaultEntry,
  VAULT_REVEAL_MS,
  type VaultEntryDraft,
  type VaultEntryPage,
  type VaultEntrySummary,
  type VaultMutationResult,
  type VaultStatus,
} from '../api/vault';
import { vaultText } from '../utils/vault/text';

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
const editing = computed(() => form.value.id !== '' || form.value.title !== '' || form.value.record_version === 0 && newEntryOpen.value);
const newEntryOpen = ref(false);
const revealedPassword = ref('');
const revealedId = ref('');
const generatedPassword = ref('');
const error = ref('');
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
  newEntryOpen.value = false;
  status.value = { ...status.value, locked: true, unlocking: false };
  busy.value = false;
}

function reportError(reason: unknown) {
  const message = reason instanceof Error ? reason.message : String(reason);
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
  clearReveal();
  form.value = emptyVaultEntry();
  newEntryOpen.value = true;
}

function cancelEdit() {
  clearReveal();
  form.value = emptyVaultEntry();
  newEntryOpen.value = false;
}

async function editEntry(id: string) {
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

async function deleteEntry() {
  if (!form.value.id || !window.confirm(vaultText('deleteConfirm'))) return;
  const operation = currentOperation();
  busy.value = true;
  const id = form.value.id;
  const expectedVersion = form.value.record_version;
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
    revealTimer = setTimeout(() => {
      if (!disposed && operation === operationEpoch) clearReveal();
    }, VAULT_REVEAL_MS);
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
    revealTimer = setTimeout(() => {
      if (!disposed && operation === operationEpoch) clearReveal();
    }, VAULT_REVEAL_MS);
  } catch (reason) {
    if (!disposed && operation === operationEpoch) reportError(reason);
  }
}

function useGenerated() {
  if (!editing.value) startNew();
  form.value.password = generatedPassword.value;
  clearReveal();
}

async function changeMasterPassword() {
  if (newMasterPassword.value !== newMasterConfirmation.value) {
    error.value = 'Master password confirmation differs';
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
.vault-panel { display: flex; flex-direction: column; gap: 14px; color: var(--macos-text-primary, #1d1d1f); }
.vault-header, .vault-toolbar, .vault-actions { display: flex; align-items: center; gap: 10px; }
.vault-header { justify-content: space-between; }
.vault-header h2 { margin: 0; font-size: 22px; }
.vault-header p, .vault-card small { margin: 4px 0 0; color: var(--macos-text-secondary, #6e6e73); }
.vault-card { padding: 16px; border: 1px solid var(--macos-border, #d8d8de); border-radius: 12px; background: var(--macos-card-bg, #fff); }
.vault-auth { max-width: 520px; display: grid; gap: 14px; }
.vault-auth label, .vault-editor label { display: grid; gap: 6px; font-size: 13px; }
.vault-panel input, .vault-panel textarea { box-sizing: border-box; width: 100%; padding: 9px 11px; border: 1px solid #c7c7cc; border-radius: 8px; background: #fff; color: #1d1d1f; }
.vault-toolbar input { flex: 1; }
.vault-button { padding: 8px 12px; border: 1px solid #c7c7cc; border-radius: 8px; background: #f5f5f7; color: #1d1d1f; cursor: pointer; }
.vault-button.primary { background: #087f5b; border-color: #087f5b; color: #fff; }
.vault-button.danger { color: #b42318; }
.vault-button:disabled { cursor: not-allowed; opacity: .58; }
.vault-layout { display: grid; grid-template-columns: minmax(220px, 1fr) minmax(300px, 2fr); gap: 14px; }
.vault-list { display: grid; gap: 8px; align-content: start; }
.vault-entry { display: grid; gap: 3px; padding: 10px; text-align: left; border: 0; border-bottom: 1px solid #e5e5ea; background: transparent; cursor: pointer; }
.vault-entry span, .vault-entry small { overflow: hidden; text-overflow: ellipsis; color: #6e6e73; }
.vault-entry .vault-reveal-action { justify-self: start; color: #087f5b; text-decoration: underline; }
.vault-editor { display: grid; gap: 12px; }
.vault-check { display: flex !important; grid-template-columns: auto 1fr; align-items: center; }
.vault-check input { width: auto; }
.vault-generated, .vault-reveal { display: flex; align-items: center; gap: 12px; }
.vault-generated code, .vault-reveal code { overflow-wrap: anywhere; }
.vault-change { display: grid; gap: 10px; }
.vault-change h3 { margin: 0; }
.vault-change-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
.vault-error { padding: 10px; border-radius: 8px; background: #fff1f0; color: #a8071a; }
.vault-notice { padding: 10px; border-radius: 8px; background: #edf8f2; color: #087f5b; }
.vault-busy { color: #6e6e73; }
@media (max-width: 760px) { .vault-layout, .vault-change-grid { grid-template-columns: 1fr; } .vault-toolbar { flex-wrap: wrap; } }
</style>
