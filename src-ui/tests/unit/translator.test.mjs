import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';

// 执行实际组件脚本，覆盖 IPC 回包与原生事件先后颠倒；不复制业务实现。
for (const [component, trigger, input, output] of [
  ['TranslatePopover', 'triggerTranslate', 'sourceText', 'translatedText'],
  ['TranslatorPanel', 'handleTranslate', 'sourceInput', 'translatedOutput'],
]) {
  test(`${component}: buffers early events, isolates requests and ignores unsolicited events`, async () => {
    const source = await readFile(new URL(`../../src/components/${component}.vue`, import.meta.url), 'utf8');
    const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
      .replace(/^import .*;$/gm, '');
    const pending = [];
    const context = vm.createContext({
      ref: (value) => ({ value }),
      computed: (get) => ({ get value() { return get(); } }),
      onMounted: () => {}, onUnmounted: () => {},
      t: (key) => key,
      invokeIpc: () => new Promise((resolve) => pending.push(resolve)),
      console,
    });
    vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
    const run = (expression) => vm.runInContext(expression, context);
    const emit = (id, type, payload) => run(`receiveStream(${JSON.stringify({ request_id: id, event: { type, payload } })})`);

    emit(1, 'done', 'another window');
    assert.equal(run(`${output}.value`), '');
    run(`${input}.value = 'source'`);
    const first = run(`${trigger}()`);
    emit(10, 'chunk', 'early');
    emit(10, 'done', 'early complete');
    assert.equal(run(`${output}.value`), '');
    pending.shift()({ request_id: 10 });
    await first;
    assert.equal(run(`${output}.value`), 'early complete');
    assert.equal(run('isTranslating.value'), false);

    const second = run(`${trigger}()`);
    emit(10, 'done', 'late old result');
    emit(11, 'chunk', 'new');
    pending.shift()({ request_id: 11 });
    await second;
    assert.equal(run(`${output}.value`), 'new');
    emit(10, 'done', 'late old result');
    assert.equal(run(`${output}.value`), 'new');
    emit(11, 'done', 'new complete');
    assert.equal(run(`${output}.value`), 'new complete');
  });
}
