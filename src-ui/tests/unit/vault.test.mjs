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
    formatOtpCode: (code) => (code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code),
    vaultText: (key) => key,
    vaultErrorText: (reason) => `localized:${String(reason)}`,
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
    auto_lock_seconds: 180, biometric_available: false, biometric_enrolled: false,
    secret_copy_available: false, capability_error: null
  }; busy.value = false`);
  return { run, unmount: () => unmounted() };
}

test('vault errors use localized keys for known failures and a localized fallback', async () => {
  const source = await readFile(new URL('../../src/utils/vault/text.ts', import.meta.url), 'utf8');
  const script = source.replace(/^import\s+[^;]+?;\s*/gm, '');
  const context = vm.createContext({ t: (key) => key, exports: {} });
  vm.runInContext(
    ts.transpileModule(script, {
      compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
    }).outputText,
    context,
  );
  const translate = (message) => context.exports.vaultErrorText(message);
  assert.equal(translate('master password unlock failed'), 'vault.errors.unlockFailed');
  assert.equal(translate('invalid vault input: title is required'), 'vault.errors.titleRequired');
  assert.equal(translate('invalid vault input: master password confirmation differs'), 'vault.errors.confirmationMismatch');
  assert.equal(translate('database operation failed: internal detail'), 'vault.errors.database');
  assert.equal(translate('Failed to fetch'), 'vault.errors.unknown');
});

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

test('VaultPanel unlocks with Touch ID without sending a password or storing secrets', async () => {
  const source = await readFile(componentUrl, 'utf8');
  assert.doesNotMatch(source, /localStorage|sessionStorage/);
  assert.match(source, /invokeIpc<VaultStatus>\('vault_unlock_biometric'\)/);
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

const totpCode = (overrides = {}) => ({
  kind: 'totp', code: '287082', digits: 6, period_secs: 30, remaining_ms: 12_000, counter: null, ...overrides,
});
const editedEntry = (overrides = {}) => ({
  id: 'fiction-otp', record_version: 1, archived: false, title: 'Fiction', url: '', category: '',
  username: '', password: 'fiction-pass', totp_secret: 'GEZDGNBVGY3TQOJQ', notes: '', ...overrides,
});

test('VaultPanel shows the backend-computed TOTP code when an entry with a secret is opened', async () => {
  const timers = [];
  const calls = [];
  const { run } = await loadComponent(async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'vault_get_entry') return editedEntry();
    if (cmd === 'vault_totp_code') return totpCode();
    return {};
  }, timers);
  await run("editEntry('fiction-otp')");
  await Promise.resolve();
  assert.equal(run('otpVisible.value'), true);
  assert.equal(run('otp.value.code'), '287082');
  assert.equal(run('otpDisplay.value'), '287 082');
  assert.equal(run('otpSeconds.value'), 12);
  assert.deepEqual(
    calls.filter(({ cmd }) => cmd === 'vault_totp_code').map(({ args }) => JSON.parse(JSON.stringify(args))),
    [{ id: 'fiction-otp' }],
  );
  // The countdown uses a 250 ms tick and refetches the code once the period is over.
  assert.equal(timers.at(-1).delay, 250);
});

test('VaultPanel refetches the TOTP code when the countdown reaches zero and stops after lock', async () => {
  const timers = [];
  let fetches = 0;
  const { run } = await loadComponent(async (cmd) => {
    if (cmd === 'vault_get_entry') return editedEntry();
    if (cmd === 'vault_totp_code') { fetches += 1; return totpCode({ remaining_ms: fetches === 1 ? 1 : 30_000 }); }
    return {};
  }, timers);
  await run("editEntry('fiction-otp')");
  await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  timers.at(-1).callback();
  await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  assert.equal(fetches, 2);
  assert.equal(run('otpRemainingMs.value') > 25_000, true);
  const pending = timers.at(-1);
  run('clearSensitiveState()');
  assert.equal(pending.cleared, true);
  assert.equal(run('otp.value'), null);
  assert.equal(run('loadedTotp.value'), '');
});

test('VaultPanel drops a late TOTP response after lock and hides the card while the secret is edited', async () => {
  const pending = [];
  const { run } = await loadComponent(async (cmd) => {
    if (cmd === 'vault_get_entry') return editedEntry();
    if (cmd === 'vault_totp_code') return new Promise((resolve) => pending.push(resolve));
    return {};
  });
  await run("editEntry('fiction-otp')");
  run('clearSensitiveState()');
  pending.shift()(totpCode());
  await Promise.resolve();
  assert.equal(run('otp.value'), null);

  run('status.value = { ...status.value, locked: false }');
  await run("editEntry('fiction-otp')");
  assert.equal(run('otpVisible.value'), true);
  run("form.value.totp_secret = 'CHANGED'");
  assert.equal(run('otpVisible.value'), false);
});

test('VaultPanel keeps an invalid TOTP secret as an inline error instead of resetting the panel', async () => {
  const { run } = await loadComponent(async (cmd) => {
    if (cmd === 'vault_get_entry') return editedEntry();
    if (cmd === 'vault_totp_code') throw new Error('one-time password secret is invalid: secret is not valid Base32');
    return {};
  });
  await run("editEntry('fiction-otp')");
  await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  assert.equal(run('otp.value'), null);
  assert.match(run('otpError.value'), /one-time password secret is invalid/);
  assert.equal(run('form.value.id'), 'fiction-otp');
  assert.equal(run('status.value.locked'), false);
});

test('VaultPanel adopts the re-encrypted secret and version after advancing an HOTP counter', async () => {
  const calls = [];
  const hotpSecret = 'otpauth://hotp/Fiction?secret=GEZDGNBVGY3TQOJQ&counter=0';
  const { run } = await loadComponent(async (cmd, args) => {
    calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? {})) });
    if (cmd === 'vault_get_entry') return editedEntry({ totp_secret: hotpSecret });
    if (cmd === 'vault_totp_code') {
      return { kind: 'hotp', code: '755224', digits: 6, period_secs: null, remaining_ms: null, counter: 0 };
    }
    if (cmd === 'vault_hotp_advance') {
      return {
        code: { kind: 'hotp', code: '287082', digits: 6, period_secs: null, remaining_ms: null, counter: 1 },
        record_version: 2,
        totp_secret: hotpSecret.replace('counter=0', 'counter=1'),
      };
    }
    return {};
  });
  await run("editEntry('fiction-otp')");
  await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  assert.equal(run('otp.value.code'), '755224');
  await run('advanceHotp()');
  assert.deepEqual(calls.find(({ cmd }) => cmd === 'vault_hotp_advance').args, { id: 'fiction-otp', expected_counter: 0 });
  assert.equal(run('otp.value.code'), '287082');
  assert.equal(run('form.value.record_version'), 2);
  assert.match(run('form.value.totp_secret'), /counter=1/);
  // Not marked as an unsaved edit: the card stays visible.
  assert.equal(run('otpVisible.value'), true);
});

test('VaultPanel copies the one-time code through the secure clipboard IPC only', async () => {
  const calls = [];
  const { run } = await loadComponent(async (cmd, args) => {
    calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? {})) });
    if (cmd === 'vault_get_entry') return editedEntry();
    if (cmd === 'vault_totp_code') return totpCode();
    return {};
  });
  await run("editEntry('fiction-otp')");
  await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  await run('copyOtp()');
  assert.deepEqual(calls.find(({ cmd }) => cmd === 'vault_copy_totp').args, { id: 'fiction-otp' });
  assert.equal(run('notice.value'), 'otpCopied');
});
