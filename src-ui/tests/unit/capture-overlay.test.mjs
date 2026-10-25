/** 执行实际蒙层初始化脚本：隐藏 WebView 永不派发动画帧时仍须完成显示握手。 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';
import * as geometry from '../../src/utils/screenshot/geometry.ts';
import * as annotations from '../../src/utils/screenshot/annotations.ts';

async function mountOverlay({ brokenFrame = false } = {}) {
  const source = await readFile(new URL('../../src/components/CaptureOverlay.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile('CaptureOverlay.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const withoutImports = ts.factory.updateSourceFile(parsed, parsed.statements.filter((node) => !ts.isImportDeclaration(node)));
  const code = ts.transpileModule(ts.createPrinter().printFile(withoutImports), {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const calls = [];
  const animationFrames = [];
  let mounted;
  const ctx = {
    clearRect() {},
    fillRect() { calls.push('shade'); },
    putImageData() { calls.push('pixels'); },
  };
  const canvas = { width: 0, height: 0, getContext: () => ctx };
  const context = vm.createContext({
    ...geometry, ...annotations,
    window: { innerWidth: 2, innerHeight: 2, addEventListener() {} },
    document: { body: { classList: { add() {} } } },
    ref: (value) => ({ value }), shallowRef: (value) => ({ value }), reactive: (value) => value,
    computed: (get) => ({ get value() { return get(); } }), watch() {},
    onMounted: (callback) => { mounted = callback; }, onUnmounted() {},
    nextTick: async () => { calls.push('dom-flush'); },
    // 模拟 WebKit 隐藏窗口：记录 rAF，但绝不执行；不能靠定时器掩盖互相等待。
    requestAnimationFrame: (callback) => animationFrames.push(callback),
    ImageData: class { constructor(pixels, width, height) { Object.assign(this, { pixels, width, height }); } },
    screenshotApi: {
      getSession: async () => ({
        session_id: 'hidden-session', frame_width: 2, frame_height: 2, frame_byte_length: 16,
        settings: { default_color: '#123456', default_stroke_width: 3 },
      }),
      overlayReady: async (id) => { assert.equal(id, 'hidden-session'); calls.push('present'); },
      cancel: async () => { calls.push('cancel'); },
    },
    fetchFrozenFrame: async () => new ArrayBuffer(brokenFrame ? 0 : 16),
    t: (key) => key, canvas, calls,
  });
  vm.runInContext(code, context);
  vm.runInContext('baseCanvas.value = canvas; uiCanvas.value = canvas; rootEl.value = { focus() { calls.push("focus"); } };', context);
  let completed = false;
  const initialization = mounted().then(() => { completed = true; });
  // 只推进普通 JS 任务；正确初始化不应等待隐藏窗口的动画帧。
  await new Promise(setImmediate);
  return { calls, animationFrames, completed, initialization, get: (expression) => vm.runInContext(expression, context) };
}

test('hidden overlay presents after writing pixels without waiting for an animation frame', async () => {
  const overlay = await mountOverlay();
  assert.equal(overlay.completed, true, 'hidden-window initialization must not wait for rAF');
  await overlay.initialization;
  assert.equal(overlay.get('loadError.value'), '');
  assert.equal(overlay.get('ready.value'), true);
  assert.ok(overlay.calls.indexOf('pixels') < overlay.calls.indexOf('present'));
  assert.ok(overlay.calls.indexOf('shade') < overlay.calls.indexOf('present'));
  assert.ok(overlay.calls.indexOf('dom-flush') < overlay.calls.indexOf('present'));
  assert.ok(overlay.calls.indexOf('present') < overlay.calls.indexOf('focus'));
  assert.ok(overlay.animationFrames.length > 0, 'normal repaint may still be queued for when visible');
});

test('invalid frozen frame shows a visible error instead of leaving a hidden session', async () => {
  const overlay = await mountOverlay({ brokenFrame: true });
  assert.equal(overlay.completed, true);
  await overlay.initialization;
  assert.equal(overlay.get('loadError.value'), 'screenshot.overlay.frameSizeMismatch');
  assert.equal(overlay.get('ready.value'), false);
  assert.deepEqual(overlay.calls, ['present']);
});
