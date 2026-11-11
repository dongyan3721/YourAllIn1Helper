import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';

const source = await readFile(new URL('../../src/utils/features.ts', import.meta.url), 'utf8');
function compile(source, context) {
  vm.runInContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
}
test('shortcut recorder uses physical letter keys and rejects IME, repeats, modifiers and bare typing', () => {
  const context = vm.createContext({});
  compile(source.replace(/export /g, ''), context);
  const run = event => vm.runInContext(`shortcutFromEvent(${JSON.stringify({ key: 'v', code: 'KeyV', ctrlKey: true, altKey: false, shiftKey: true, metaKey: false, isComposing: false, repeat: false, ...event })})`, context);
  assert.equal(run({ key: 'м' }), 'Ctrl+Shift+V');
  assert.equal(run({ key: 'F8', code: 'F8', ctrlKey: false }), 'Shift+F8');
  assert.equal(run({ ctrlKey: false, metaKey: true }), 'Shift+Super+V');
  for (const event of [{ isComposing: true }, { repeat: true }, { key: 'Control' }, { key: 'Tab' }, { key: 'Escape' }, { key: 'CapsLock', code: 'CapsLock' }, { ctrlKey: false, shiftKey: false }, { code: 'IntlBackslash', key: '<' }]) assert.equal(run(event), null);
  assert.equal(vm.runInContext("displayRecordedShortcut('Ctrl+Alt+P', 'windows')", context), 'Ctrl + Alt + P');
  assert.equal(vm.runInContext("displayRecordedShortcut('Shift+Super+V', 'macos')", context), '⇧ + ⌘ + V');
});

async function preferencesHarness(invoke) {
  const component = await readFile(new URL('../../src/components/PreferencesPanel.vue', import.meta.url), 'utf8');
  const script = component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/import\s+[\s\S]*?from\s+['"][^'"]+['"];?/g, '');
  const callbacks = [];
  const context = vm.createContext({
    ref: value => ({ value }), computed: fn => ({ get value() { return fn(); } }),
    defineProps: () => ({}), defineEmits: () => () => {}, onMounted: () => {}, onUnmounted: fn => callbacks.push(fn),
    invokeIpc: invoke, t: key => key,
  });
  compile(source.replace(/export /g, ''), context);
  compile(script, context);
  return { run: expr => vm.runInContext(expr, context), callbacks };
}
const row = { spec: { action_id: 'clipboard:toggle_popup', module_id: 'clipboard', description: 'Clipboard', default_logical_hotkey: 'PrimaryMod+Shift+V' }, display_label: 'Ctrl + Shift + V', enabled: true, editable: true };
test('shortcut save failure preserves old binding and recorded draft; reset uses explicit null', async () => {
  const calls = [];
  let reject = true;
  const harness = await preferencesHarness(async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'kernel_update_shortcut' && reject) throw new Error('conflict with another action');
    return { shortcuts: [{ ...row, display_label: 'Ctrl + Shift + F8' }] };
  });
  const { run } = harness;
  run(`shortcuts.value = [${JSON.stringify(row)}]; draft.value['clipboard:toggle_popup'] = 'Ctrl+Shift+F8'`);
  await run("saveShortcut(shortcuts.value[0], draft.value['clipboard:toggle_popup'])");
  assert.equal(run('shortcuts.value[0].display_label'), row.display_label);
  assert.equal(run("draft.value['clipboard:toggle_popup']"), 'Ctrl+Shift+F8');
  assert.equal(run('failed.value'), true);
  assert.equal(run('saving.value'), null);
  reject = false;
  await run('saveShortcut(shortcuts.value[0], null)');
  assert.equal(calls.at(-1).args.hotkey, null);
  assert.equal(run("draft.value['clipboard:toggle_popup']"), undefined);
  assert.equal(run('shortcuts.value[0].display_label'), 'Ctrl + Shift + F8');
});
test('recorder focus and blur serialize native suppression, including unmount cleanup', async () => {
  const calls = [];
  const pending = [];
  const { run, callbacks } = await preferencesHarness((cmd, args) => {
    calls.push({ cmd, args });
    return new Promise(resolve => pending.push(resolve));
  });
  run("setRecording('screenshot:capture'); setRecording(null); setRecording('clipboard:toggle_popup')");
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls.map(c => c.args.recording), [true]);
  pending.shift()({}); await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls.map(c => c.args.recording), [true, false]);
  pending.shift()({}); await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls.map(c => c.args.recording), [true, false, true]);
  pending.shift()({}); await run('recordingQueue');
  callbacks[0](); await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls.at(-1).args.recording, false);
  pending.shift()({}); await run('recordingQueue');
});
test('registered native shortcut events affect only the actively recording row', async () => {
  const { run } = await preferencesHarness(async () => ({}));
  run("acceptRecordedShortcut('F1')");
  assert.equal(run('Object.keys(draft.value).length'), 0);
  run("setRecording('clipboard:toggle_popup'); acceptRecordedShortcut('F1')");
  assert.equal(run("draft.value['clipboard:toggle_popup']"), 'F1');
  assert.equal(run('recording.value'), null);
  run("acceptRecordedShortcut('Ctrl+Shift+P')");
  assert.equal(run("draft.value['clipboard:toggle_popup']"), 'F1');
  await run('recordingQueue');
});
