import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

async function load(invokeIpc) {
  const source = await readFile(new URL('../../src/components/ClipboardPopup.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/^import\s+[^;]+?;\s*/gm, '');
  const listeners = {};
  const watchers = [];
  const hooks = { mounted: [], unmount: [] };
  const context = vm.createContext({
    ref: value => ({ value }), computed: fn => ({ get value() { return fn(); } }),
    watch: (_source, callback) => { watchers.push(callback); },
    onMounted: callback => { hooks.mounted.push(callback); },
    onBeforeUnmount: callback => { hooks.unmount.push(callback); },
    nextTick: callback => { callback?.(); return Promise.resolve(); },
    window: {
      addEventListener: (name, handler) => { listeners[name] = handler; },
      removeEventListener: (name, handler) => { if (listeners[name] === handler) delete listeners[name]; },
    },
    document: { querySelector: () => null },
    t: key => key,
    invokeIpc, console: { error: () => {} },
  });
  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  const run = expression => vm.runInContext(expression, context);
  run.listeners = listeners;
  run.watchers = watchers;
  run.hooks = hooks;
  return run;
}

function keyEvent(key, extra = {}) {
  return {
    key, isComposing: false, defaultPrevented: false, target: null,
    preventDefault() { this.defaultPrevented = true; },
    ...extra,
  };
}

const records = [1, 2, 3].map(id => ({
  id, item_type: 'text', content_text: `candidate-${id}`, source_app_name: null,
  is_pinned: false, updated_at: id,
}));

/** 模拟弹窗挂载：监听器登记完毕、历史列表加载完成。 */
async function mounted(calls = []) {
  const run = await load(async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'clipboard_list_items') return records;
    return {};
  });
  for (const hook of run.hooks.mounted) await hook();
  return run;
}

test('clipboard popup displays paste failure and permits retry', async () => {
  let denied = true;
  const run = await load(async () => { if (denied) throw new Error('authorize current signed app'); });
  await run('pasteItem(1)');
  assert.match(run('pasteError.value'), /authorize current signed app/);
  assert.equal(run('pasting.value'), false);
  denied = false;
  await run('pasteItem(1)');
  assert.equal(run('pasteError.value'), '');
  assert.equal(run('pasting.value'), false);
});

test('clipboard popup submits only one paste while an operation is pending', async () => {
  let finish;
  let calls = 0;
  const run = await load(() => { calls++; return new Promise(resolve => { finish = resolve; }); });
  const first = run('pasteItem(1)');
  await run('pasteItem(2)');
  assert.equal(calls, 1);
  finish({});
  await first;
  assert.equal(run('pasting.value'), false);
});

test('arrow keys move the selection from a window-level listener even when no element has focus', async () => {
  const run = await mounted();
  const keydown = run.listeners.keydown;
  assert.equal(typeof keydown, 'function');

  const down = keyEvent('ArrowDown');
  keydown(down);
  assert.equal(run('selectedIndex.value'), 1);
  assert.equal(down.defaultPrevented, true);
  keydown(keyEvent('ArrowDown'));
  keydown(keyEvent('ArrowDown'));
  assert.equal(run('selectedIndex.value'), 0, 'wraps from the last item to the first');
  keydown(keyEvent('ArrowUp'));
  assert.equal(run('selectedIndex.value'), 2, 'wraps from the first item to the last');
});

test('enter pastes the highlighted candidate exactly once', async () => {
  const calls = [];
  const run = await mounted(calls);
  run.listeners.keydown(keyEvent('ArrowDown'));
  run.listeners.keydown(keyEvent('Enter'));
  await Promise.resolve();
  const pastes = calls.filter(([cmd]) => cmd === 'clipboard_paste_item');
  // vm 上下文内创建的对象原型不同，按 JSON 形状比较。
  assert.equal(JSON.stringify(pastes), JSON.stringify([['clipboard_paste_item', { id: 2, as_plain_text: false }]]));

  // The search box handles Enter itself and prevents default before the event reaches window.
  const fromInput = keyEvent('Enter', { defaultPrevented: true });
  run.listeners.keydown(fromInput);
  await Promise.resolve();
  assert.equal(calls.filter(([cmd]) => cmd === 'clipboard_paste_item').length, 1);
});

test('enter on a focused button and IME composition keys are not hijacked', async () => {
  const calls = [];
  const run = await mounted(calls);
  const button = keyEvent('Enter', { target: { closest: selector => (selector === 'button' ? {} : null) } });
  run.listeners.keydown(button);
  assert.equal(button.defaultPrevented, false);

  const composing = keyEvent('ArrowDown', { isComposing: true });
  run.listeners.keydown(composing);
  assert.equal(composing.defaultPrevented, false);
  assert.equal(run('selectedIndex.value'), 0);
  assert.equal(calls.some(([cmd]) => cmd === 'clipboard_paste_item'), false);
});

test('escape destroys the popup even when the list is empty', async () => {
  const calls = [];
  const run = await load(async (cmd, args) => {
    calls.push([cmd, args]);
    return cmd === 'clipboard_list_items' ? [] : {};
  });
  for (const hook of run.hooks.mounted) await hook();
  run.listeners.keydown(keyEvent('ArrowDown'));
  assert.equal(run('selectedIndex.value'), 0);
  run.listeners.keydown(keyEvent('Escape'));
  await Promise.resolve();
  assert.equal(JSON.stringify(calls.at(-1)), JSON.stringify(['kernel_destroy_window', { label: 'clipboard-popup' }]));
});

test('window activation returns keyboard focus to the search box', async () => {
  const run = await mounted();
  let focused = 0;
  run('searchInputRef.value = { focus() { globalThis.__focused = (globalThis.__focused || 0) + 1; } }');
  run.listeners.focus();
  run.listeners.focus();
  focused = run('globalThis.__focused');
  assert.equal(focused, 2);
});

test('changing the search text resets an out-of-range highlight', async () => {
  const run = await mounted();
  run.listeners.keydown(keyEvent('ArrowDown'));
  run.listeners.keydown(keyEvent('ArrowDown'));
  assert.equal(run('selectedIndex.value'), 2);
  run('searchQuery.value = "candidate-1"');
  for (const watcher of run.watchers) watcher();
  assert.equal(run('selectedIndex.value'), 0);
});

test('unmounting removes the global key and focus listeners', async () => {
  const run = await mounted();
  assert.ok(run.listeners.keydown && run.listeners.focus);
  for (const hook of run.hooks.unmount) hook();
  assert.equal(run.listeners.keydown, undefined);
  assert.equal(run.listeners.focus, undefined);
});
