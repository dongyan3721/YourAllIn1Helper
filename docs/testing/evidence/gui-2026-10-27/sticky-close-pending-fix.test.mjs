import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';

const workspace = process.cwd();
const require = createRequire(workspace + '/src-ui/package.json');
const ts = require('typescript');

async function component() {
  const source = await readFile(workspace + '/src-ui/src/components/StickyNote.vue', 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script, 'must load the actual component script');
  const parsed = ts.createSourceFile('StickyNote.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const body = ts.factory.updateSourceFile(parsed, parsed.statements.filter(node => !ts.isImportDeclaration(node)));
  const code = ts.transpileModule(ts.createPrinter().printFile(body), {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText;

  const calls = [];
  const timers = new Map();
  let nextTimer = 0;
  let destroyed = false;
  const context = vm.createContext({
    ref: value => ({ value }),
    computed: get => ({ get value() { return get(); } }),
    defineProps: () => ({}),
    onMounted() {},
    t: key => key,
    setTimeout: (fn, delay) => {
      assert.equal(delay, 400, 'current component debounce is 400 ms');
      const id = ++nextTimer;
      timers.set(id, fn);
      return id;
    },
    clearTimeout: id => timers.delete(id),
    invokeIpc: async (name, payload) => {
      calls.push({ name, id: payload.id, title: payload.title, content: payload.content_md });
      if (name === 'sticky_close_note') {
        destroyed = true;
        timers.clear(); // Native close destroys the WebView and its pending callbacks.
      }
    },
    console,
  });
  vm.runInContext(code, context);
  vm.runInContext(
    'note.value.id = "qa-note"; note.value.title = "QA fictional title"; note.value.content_md = "QA fictional content";',
    context,
  );
  return {
    calls,
    get pendingTimers() { return timers.size; },
    get destroyed() { return destroyed; },
    scheduleSave: () => vm.runInContext('saveContentDebounced()', context),
    close: () => vm.runInContext('closeNoteWindow()', context),
    flushTimer: async () => {
      const [id, callback] = timers.entries().next().value;
      timers.delete(id);
      await callback();
    },
  };
}

test('ordinary debounce saves the fictional note when its timer fires', async () => {
  const c = await component();
  c.scheduleSave();
  assert.equal(c.pendingTimers, 1);
  await c.flushTimer();
  assert.deepEqual(c.calls.map(call => call.name), ['sticky_save_content']);
  assert.equal(c.calls[0].title, 'QA fictional title');
  assert.equal(c.calls[0].content, 'QA fictional content');
});

test('closing while a save is pending must persist edits before destroying the WebView', async () => {
  const c = await component();
  c.scheduleSave();
  assert.equal(c.pendingTimers, 1, 'the edit must be pending before close');
  await c.close();
  assert.equal(c.destroyed, true);
  assert.deepEqual(c.calls.map(call => call.name), ['sticky_save_content', 'sticky_close_note'],
    'pending edits must be saved successfully before close');
  assert.equal(c.calls[0].title, 'QA fictional title');
  assert.equal(c.calls[0].content, 'QA fictional content');
});
