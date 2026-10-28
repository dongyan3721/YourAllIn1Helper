import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

async function load(invokeIpc) {
  const source = await readFile(new URL('../../src/components/ClipboardPopup.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/^import\s+[^;]+?;\s*/gm, '');
  const context = vm.createContext({
    ref: value => ({ value }), computed: fn => ({ get value() { return fn(); } }),
    onMounted: () => {}, nextTick: async () => {}, t: key => key,
    invokeIpc, console: { error: () => {} },
  });
  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  return expression => vm.runInContext(expression, context);
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
