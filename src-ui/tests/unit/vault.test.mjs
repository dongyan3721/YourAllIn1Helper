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

for (const [action, field, secret] of [
  ["revealPassword('fiction-entry')", 'revealedPassword', 'fiction-reveal'],
  ['generatePassword()', 'generatedPassword', 'fiction-generated'],
]) {
  test(`${action} expires after refresh and search change the operation epoch`, async () => {
    const timers = [];
    const { run } = await loadComponent((cmd) => {
      if (cmd === 'vault_reveal_password') return Promise.resolve('fiction-reveal');
      if (cmd === 'vault_generate_password') return Promise.resolve('fiction-generated');
      if (cmd === 'vault_list_entries') return Promise.resolve({ entries: [] });
      return Promise.resolve({});
    }, timers);
    await run(action);
    const timer = timers.at(-1);
    assert.equal(run(`${field}.value`), secret);
    run(`searchQuery.value = 'fiction-search'`);
    await run('refreshEntries()');
    assert.equal(timer.cleared, false);
    timer.callback();
    assert.equal(run(`${field}.value`), '');
  });
}

test('VaultPanel keeps the latest secret when reveals are consecutive and a stale timer fires', async () => {
  const timers = [];
  const { run } = await loadComponent((cmd, args) => {
    if (cmd === 'vault_reveal_password') return Promise.resolve(`fiction-${args.id}`);
    return Promise.resolve({});
  }, timers);
  await run(`revealPassword('first')`);
  const firstTimer = timers.at(-1);
  await run(`revealPassword('second')`);
  const secondTimer = timers.at(-1);
  assert.equal(firstTimer.cleared, true);
  assert.equal(run('revealedPassword.value'), 'fiction-second');
  firstTimer.callback();
  assert.equal(run('revealedPassword.value'), 'fiction-second');
  secondTimer.callback();
  assert.equal(run('revealedPassword.value'), '');
});

test('VaultPanel ignores an out-of-order reveal response without replacing the latest secret or timer', async () => {
  const pending = new Map();
  const timers = [];
  const { run } = await loadComponent((cmd, args) => {
    if (cmd === 'vault_reveal_password') {
      return new Promise((resolve) => pending.set(args.id, resolve));
    }
    return Promise.resolve({});
  }, timers);
  const first = run(`revealPassword('first')`);
  const second = run(`revealPassword('second')`);
  pending.get('second')('fiction-second');
  await second;
  pending.get('first')('fiction-first');
  await first;
  assert.equal(run('revealedPassword.value'), 'fiction-second');
  assert.equal(run('revealedId.value'), 'second');
  assert.equal(timers.length, 1);
  timers[0].callback();
  assert.equal(run('revealedPassword.value'), '');
});

for (const [action, field, secret] of [
  ["revealPassword('fiction-entry')", 'revealedPassword', 'fiction-reveal'],
  ['generatePassword()', 'generatedPassword', 'fiction-generated'],
]) {
  for (const cleanup of ['lockNow', 'unmount']) {
    test(`${action} timer is cancelled by ${cleanup}`, async () => {
      const timers = [];
      const { run, unmount } = await loadComponent((cmd) => {
        if (cmd === 'vault_reveal_password') return Promise.resolve('fiction-reveal');
        if (cmd === 'vault_generate_password') return Promise.resolve('fiction-generated');
        return Promise.resolve({});
      }, timers);
      await run(action);
      const timer = timers.at(-1);
      assert.equal(run(`${field}.value`), secret);
      if (cleanup === 'unmount') unmount();
      else run('lockNow()');
      assert.equal(timer.cleared, true);
      assert.equal(run(`${field}.value`), '');
      timer.callback();
      assert.equal(run(`${field}.value`), '');
    });
  }
}

test('VaultPanel never persists secrets or enables M2/clipboard IPC', async () => {
  const source = await readFile(componentUrl, 'utf8');
  assert.doesNotMatch(source, /localStorage|sessionStorage|vault_unlock_biometric/);
  assert.match(source, /vault_copy_password/);
  assert.match(source, /pagehide/);
  assert.match(source, /visibilitychange/);
  assert.match(source, /onUnmounted/);
});

test('VaultPanel transfers generated password into a newly opened editor before clearing it', async () => {
  const timers = [];
  const { run } = await loadComponent(async (cmd) => {
    if (cmd === 'vault_generate_password') return 'fiction-generated-new';
    return {};
  }, timers);
  assert.equal(run('editing.value'), false);
  await run('generatePassword()');
  run('useGenerated()');
  assert.equal(run('editing.value'), true);
  assert.equal(run('form.value.password'), 'fiction-generated-new');
  assert.equal(run('generatedPassword.value'), '');
  assert.equal(timers.at(-1).cleared, true);
});

test('VaultPanel transfers generated password without resetting an existing entry', async () => {
  const { run } = await loadComponent(async (cmd) => {
    if (cmd === 'vault_generate_password') return 'fiction-generated-existing';
    return {};
  });
  run("form.value.id = 'fiction-existing'; form.value.title = 'Keep title'; form.value.record_version = 3");
  await run('generatePassword()');
  run('useGenerated()');
  assert.equal(run('form.value.password'), 'fiction-generated-existing');
  assert.equal(run('form.value.id'), 'fiction-existing');
  assert.equal(run('form.value.title'), 'Keep title');
  assert.equal(run('form.value.record_version'), 3);
  assert.equal(run('generatedPassword.value'), '');
});

test('VaultPanel does not delete before an explicit confirmation request', async () => {
  const calls = [];
  const { run } = await loadComponent(async (cmd) => {
    calls.push(cmd);
    return cmd === 'vault_list_entries' ? { entries: [] } : {};
  });
  run("form.value.id = 'fiction-delete'; form.value.record_version = 4");
  await run('deleteEntry()');
  assert.deepEqual(calls, []);
  assert.equal(run('form.value.id'), 'fiction-delete');
});

test('VaultPanel cancels pending deletion and rejects a changed target or version', async () => {
  const calls = [];
  const { run } = await loadComponent(async (cmd) => { calls.push(cmd); return {}; });
  run("form.value.id = 'fiction-delete'; form.value.record_version = 4; requestDelete(); cancelDelete()");
  await run('deleteEntry()');
  run("requestDelete(); form.value.id = 'fiction-other'");
  await run('deleteEntry()');
  run('requestDelete(); form.value.record_version = 5');
  await run('deleteEntry()');
  assert.deepEqual(calls, []);
});

test('VaultPanel deletes the confirmed version once and closes the editor', async () => {
  const calls = [];
  const { run } = await loadComponent(async (cmd, args) => {
    calls.push({ cmd, args });
    return cmd === 'vault_list_entries' ? { entries: [] } : {};
  });
  run("form.value.id = 'fiction-delete'; form.value.record_version = 4; requestDelete()");
  await run('deleteEntry()');
  await run('deleteEntry()');
  assert.deepEqual(calls.filter(({ cmd }) => cmd === 'vault_delete_entry').map(({ args }) => JSON.parse(JSON.stringify(args))), [
    { id: 'fiction-delete', expected_version: 4 },
  ]);
  assert.equal(run('deleteConfirmation.value'), null);
  assert.equal(run('editing.value'), false);
});

test('VaultPanel clears delete confirmation on lock and editor replacement', async () => {
  const { run } = await loadComponent(async () => ({}));
  for (const cleanup of ['clearSensitiveState()', 'startNew()', 'cancelEdit()']) {
    run("form.value.id = 'fiction-delete'; form.value.record_version = 4; requestDelete()");
    run(cleanup);
    assert.equal(run('deleteConfirmation.value'), null);
  }
});
