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
    invokeIpc: invoke, t: key => key, notify: () => {},
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
    if (cmd === 'kernel_check_shortcut') return { action_id: args.action_id, hotkey: args.hotkey, status: 'available' };
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
test('conflict preflight blocks persistence and preserves the draft and active binding', async () => {
  const calls = [];
  const { run } = await preferencesHarness(async (cmd, args) => {
    calls.push({ cmd, args });
    return { action_id: args.action_id, status: 'internal_conflict', conflicts: [row] };
  });
  run(`shortcuts.value = [${JSON.stringify(row)}]; draft.value['clipboard:toggle_popup'] = 'F1'`);
  await run("saveShortcut(shortcuts.value[0], 'F1')");
  assert.equal(calls.some(call => call.cmd === 'kernel_update_shortcut'), false);
  assert.equal(run("draft.value['clipboard:toggle_popup']"), 'F1');
  assert.equal(run('shortcuts.value[0].display_label'), row.display_label);
  assert.equal(run('failed.value'), true);
  assert.equal(run('saving.value'), null);
});
test('late validation never overwrites the result for a newer recorded shortcut', async () => {
  const pending = [];
  const { run } = await preferencesHarness((cmd) => cmd === 'kernel_check_shortcut'
    ? new Promise(resolve => pending.push(resolve)) : Promise.resolve({}));
  run("setRecording('clipboard:toggle_popup'); acceptRecordedShortcut('F1'); setRecording('clipboard:toggle_popup'); acceptRecordedShortcut('Ctrl+Shift+F8')");
  pending[1]({ action_id: 'clipboard:toggle_popup', status: 'available' });
  await new Promise(resolve => setImmediate(resolve));
  pending[0]({ action_id: 'clipboard:toggle_popup', status: 'internal_conflict' });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(run("checks.value['clipboard:toggle_popup'].status"), 'available');
  assert.equal(run("draft.value['clipboard:toggle_popup']"), 'Ctrl+Shift+F8');
  assert.equal(run("checking.value['clipboard:toggle_popup']"), false);
});
test('a bulk scan includes unsaved drafts and drops stale scans when the user edits', async () => {
  const pending = [];
  const { run } = await preferencesHarness((cmd, args) => cmd === 'kernel_check_shortcuts'
    ? new Promise(resolve => pending.push(resolve))
    : Promise.resolve({ action_id: args.action_id, status: 'available' }));
  const scan = run('checkAllShortcuts()');
  run("setRecording('clipboard:toggle_popup'); acceptRecordedShortcut('Ctrl+Shift+F8')");
  pending.shift()({ checks: [{ action_id: 'clipboard:toggle_popup', status: 'os_unavailable' }] });
  await scan;
  assert.equal(run('scanSummary.value'), null);
  assert.equal(run("checks.value['clipboard:toggle_popup'].status"), 'available');
  const second = run('checkAllShortcuts()');
  pending.shift()({ checks: [{ action_id: 'clipboard:toggle_popup', status: 'os_unavailable' }] });
  await second;
  assert.equal(run('scanSummary.value.issues'), 0);
  assert.equal(run('scanSummary.value.count'), 1);
});


test('feature captions follow configured platform labels and drop hints for unassigned keys', async () => {
  const source = await readFile(new URL('../../src/utils/actionShortcuts.ts', import.meta.url), 'utf8');
  const mounted = [], unmounted = [], pending = [];
  let update;
  const context = vm.createContext({
    ref: value => ({ value }), onMounted: fn => mounted.push(fn), onUnmounted: fn => unmounted.push(fn),
    navigator: { platform: 'Win32' },
    window: { addEventListener() {}, removeEventListener() {}, __TAURI__: { event: { listen: async (_, fn) => { update = fn; return () => {}; } } } },
    invokeIpc: () => new Promise(resolve => pending.push(resolve)),
  });
  compile(source.replace(/^import .*;$/gm, '').replace(/export /g, ''), context);
  vm.runInContext('var hints = useActionShortcuts()', context);
  await mounted[0]();
  assert.equal(vm.runInContext("hints.caption('Open', 'clipboard:toggle_popup')", context), 'Open');
  pending.shift()({ shortcuts: [{ spec: { action_id: 'clipboard:toggle_popup' }, display_label: 'Ctrl + Alt + D' }] });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(vm.runInContext("hints.caption('Open', 'clipboard:toggle_popup')", context), 'Open (Ctrl + Alt + D)');
  const refresh = update();
  pending.shift()({ shortcuts: [{ spec: { action_id: 'clipboard:toggle_popup' }, display_label: '' }] });
  await refresh;
  assert.equal(vm.runInContext("hints.caption('Open', 'clipboard:toggle_popup')", context), 'Open');
  assert.equal(vm.runInContext("localShortcut('PrimaryMod+S')", context), 'Ctrl+S');
  vm.runInContext("navigator.platform = 'MacIntel'", context);
  assert.equal(vm.runInContext("localShortcut('PrimaryMod+S')", context), '⌘+S');
  const late = update();
  unmounted[0]();
  pending.shift()({ shortcuts: [{ spec: { action_id: 'clipboard:toggle_popup' }, display_label: 'stale' }] });
  await late;
  assert.equal(vm.runInContext("hints.caption('Open', 'clipboard:toggle_popup')", context), 'Open');
});


test('window pin list combines live OS style and managed pins, then clears managed state after unpin', async () => {
  const component = await readFile(new URL('../../src/components/FeaturePanel.vue', import.meta.url), 'utf8');
  const script = component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/import\s+[\s\S]*?from\s+['"][^'"]+['"];?/g, '');
  let pinned = [{ info: { handle: 7 } }];
  const context = vm.createContext({
    ref: value => ({ value }), computed: fn => ({ get value() { return fn(); } }),
    defineAsyncComponent: fn => fn, onMounted() {}, onUnmounted() {}, URLSearchParams,
    window: { location: { search: '?window=feature-window_pin' } },
    useActionShortcuts: () => ({ caption: text => text }),
    t: key => key,
    invokeIpc: async cmd => {
      if (cmd === 'kernel_get_status') return { module_catalog: [{ id: 'window_pin', enabled: true, supported: true }] };
      if (cmd === 'winpin_list_candidates') return { candidates: [{ handle: 7, is_topmost: false }, { handle: 8, is_topmost: true }, { handle: 9, is_topmost: false }], pinned };
      throw Error(cmd);
    },
  });
  compile(source.replace(/export /g, ''), context);
  compile(script, context);
  await vm.runInContext('refreshDashboard()', context);
  assert.deepEqual(JSON.parse(vm.runInContext('JSON.stringify(externalWindows.value.map(w => w.is_topmost))', context)), [true, true, false]);
  pinned = [];
  await vm.runInContext('refreshDashboard()', context);
  assert.deepEqual(JSON.parse(vm.runInContext('JSON.stringify(externalWindows.value.map(w => w.is_topmost))', context)), [false, true, false]);
});
