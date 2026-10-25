import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';

const componentUrl = new URL('../../src/components/VaultPanel.vue', import.meta.url);

async function loadComponent(invokeIpc, timers = []) {
  const source = await readFile(componentUrl, 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import\s+[^;]+?;\s*/gm, '');
  let unmounted;
  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    onMounted: () => {},
    onUnmounted: (callback) => { unmounted = callback; },
    invokeIpc,
    emptyVaultEntry: () => ({
      id: '', record_version: 0, archived: false, title: '', url: '', category: '',
      username: '', password: '', totp_secret: '', notes: '',
    }),
    VAULT_REVEAL_MS: 15_000,
    vaultText: (key) => key,
    setTimeout: (callback, delay) => {
      const timer = { callback, delay, cleared: false };
      timers.push(timer);
      return timer;
    },
    clearTimeout: (timer) => { if (timer) timer.cleared = true; },
    window: {
      addEventListener: () => {}, removeEventListener: () => {}, confirm: () => true,
    },
    document: { addEventListener: () => {}, removeEventListener: () => {}, hidden: false },
  });
  vm.runInContext(
    ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText,
    context,
  );
  const run = (expression) => vm.runInContext(expression, context);
  run(`status.value = {
    initialized: true, locked: false, unlocking: false, epoch: 7,
    auto_lock_seconds: 180, biometric_available: false,
    secret_copy_available: false, capability_error: null
  }; busy.value = false`);
  return { run, unmount: () => unmounted() };
}

test('VaultPanel clears every local secret on unmount and requests backend lock', async () => {
  const calls = [];
  const { run, unmount } = await loadComponent((cmd) => {
    calls.push(cmd);
    return Promise.resolve({});
  });
  run(`masterPassword.value = 'fiction-master';
    oldMasterPassword.value = 'fiction-old'; newMasterPassword.value = 'fiction-new';
    revealedPassword.value = 'fiction-reveal'; generatedPassword.value = 'fiction-generated';
    form.value.password = 'fiction-edit'; entries.value = [{ title: 'fiction-list' }]`);
  unmount();
  assert.equal(run('masterPassword.value'), '');
  assert.equal(run('oldMasterPassword.value'), '');
  assert.equal(run('newMasterPassword.value'), '');
  assert.equal(run('revealedPassword.value'), '');
  assert.equal(run('generatedPassword.value'), '');
  assert.equal(run('form.value.password'), '');
  assert.equal(run('entries.value.length'), 0);
  assert.equal(run('status.value.locked'), true);
  assert.deepEqual(calls, ['vault_lock']);
});

test('VaultPanel drops a late reveal result after lock or route cleanup', async () => {
  const pending = [];
  const { run } = await loadComponent((cmd) => {
    if (cmd === 'vault_reveal_password') return new Promise((resolve) => pending.push(resolve));
    return Promise.resolve({});
  });
  const reveal = run(`revealPassword('fiction-entry')`);
  run('clearSensitiveState()');
  pending.shift()('late-fiction-secret');
  await reveal;
  assert.equal(run('revealedPassword.value'), '');
  assert.equal(run('revealedId.value'), '');
});

test('VaultPanel expires reveal and generated values with the same 15 second timer contract', async () => {
  const timers = [];
  const { run } = await loadComponent((cmd) => {
    if (cmd === 'vault_reveal_password') return Promise.resolve('fiction-reveal');
    if (cmd === 'vault_generate_password') return Promise.resolve('fiction-generated');
    return Promise.resolve({});
  }, timers);
  await run(`revealPassword('fiction-entry')`);
  assert.equal(run('revealedPassword.value'), 'fiction-reveal');
  assert.equal(timers.at(-1).delay, 15_000);
  timers.at(-1).callback();
  assert.equal(run('revealedPassword.value'), '');

  await run('generatePassword()');
  assert.equal(run('generatedPassword.value'), 'fiction-generated');
  assert.equal(timers.at(-1).delay, 15_000);
  timers.at(-1).callback();
  assert.equal(run('generatedPassword.value'), '');
});

test('VaultPanel never persists secrets or enables M2/clipboard IPC', async () => {
  const source = await readFile(componentUrl, 'utf8');
  assert.doesNotMatch(source, /localStorage|sessionStorage|vault_unlock_biometric/);
  assert.match(source, /vault_copy_password/);
  assert.match(source, /pagehide/);
  assert.match(source, /visibilitychange/);
  assert.match(source, /onUnmounted/);
});
