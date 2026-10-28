import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';
import { createRequire } from 'node:module';

const workspace = process.cwd();
const require = createRequire(workspace + '/src-ui/package.json');
const ts = require('typescript');

async function fixture() {
  const source = await fs.readFile(workspace + '/src-ui/src/components/VaultPanel.vue', 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import\s+[^;]+?;\s*/gm, '');
  const timers = [];
  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    onMounted() {}, onUnmounted() {},
    emptyVaultEntry: () => ({ id: '', record_version: 0, archived: false, title: '',
      url: '', category: '', username: '', password: '', totp_secret: '', notes: '' }),
    VAULT_REVEAL_MS: 15_000,
    vaultText: (key) => key,
    setTimeout: (callback, delay) => {
      const timer = { callback, delay, cleared: false };
      timers.push(timer);
      return timer;
    },
    clearTimeout: (timer) => { if (timer) timer.cleared = true; },
    invokeIpc: async (cmd) => {
      if (cmd === 'vault_reveal_password') return 'fiction-revealed-password';
      if (cmd === 'vault_generate_password') return 'fiction-generated-password';
      if (cmd === 'vault_list_entries') return { entries: [] };
      return {};
    },
    window: { addEventListener() {}, removeEventListener() {}, confirm: () => true },
    document: { addEventListener() {}, removeEventListener() {}, hidden: false },
  });
  vm.runInContext(ts.transpileModule(script, {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText, context);
  const run = (expression) => vm.runInContext(expression, context);
  run('status.value = { initialized: true, locked: false, unlocking: false, epoch: 7 }; busy.value = false');
  return { run, timers };
}

for (const [action, displayedField, synthetic] of [
  ["revealPassword('fiction-entry')", 'revealedPassword', 'fiction-revealed-password'],
  ['generatePassword()', 'generatedPassword', 'fiction-generated-password'],
]) {
  test(action + ' clears after 15s even if the list is refreshed', async () => {
    const { run, timers } = await fixture();
    await run(action);
    assert.equal(run(displayedField + '.value'), synthetic);
    assert.equal(timers.length, 1);
    assert.equal(timers[0].delay, 15_000);
    await run('refreshEntries()');
    assert.equal(timers[0].cleared, false, 'refresh must not cancel the expiration timer');
    timers[0].callback();
    assert.equal(run(displayedField + '.value'), '', 'secret must be cleared after 15 seconds');
  });
}
