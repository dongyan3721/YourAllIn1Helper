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

test('TranslatePopover: openDashboardSettings invokes kernel_open_dashboard with /translator route', async () => {
  const source = await readFile(new URL('../../src/components/TranslatePopover.vue', import.meta.url), 'utf8');
  assert.match(source, /v-if="missingApiKey"/, 'missingApiKey alert should use v-if to coexist with permissionDenied');
  assert.doesNotMatch(source, /v-else-if="missingApiKey"/, 'missingApiKey must not be blocked by permissionDenied');
  assert.doesNotMatch(source, /kernel_open_window/, 'must not reference nonexistent kernel_open_window');

  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import .*;$/gm, '');

  const ipcCalls = [];
  const errorsLogged = [];
  let shouldFailIpc = false;

  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    onMounted: () => {}, onUnmounted: () => {},
    t: (key) => key,
    invokeIpc: async (cmd, args) => {
      ipcCalls.push({ cmd, args });
      if (shouldFailIpc) throw new Error('IPC failed');
      return { active_window_count: 1 };
    },
    console: {
      ...console,
      error: (...args) => errorsLogged.push(args.join(' ')),
    },
  });

  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  const run = (expression) => vm.runInContext(expression, context);

  // 1. 成功调用验证路由契约
  await run('openDashboardSettings()');
  assert.equal(ipcCalls.length, 1);
  assert.equal(ipcCalls[0].cmd, 'kernel_open_dashboard');
  assert.deepEqual(JSON.parse(JSON.stringify(ipcCalls[0].args)), { route: '/translator' });

  // 2. 失败时捕获并记录日志，不向外抛出未捕获 Promise 异常
  shouldFailIpc = true;
  await run('openDashboardSettings()');
  assert.equal(ipcCalls.length, 2);
  assert.ok(errorsLogged.some((msg) => msg.includes('打开主控台翻译设置失败')));
});

test('TranslatePopover: detects missing API Key and coexists with permissionDenied alert', async () => {
  const source = await readFile(new URL('../../src/components/TranslatePopover.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import .*;$/gm, '');

  let providerHasKey = false;
  let mockSelection = {
    permission_denied: true,
    guidance: '请在「系统设置 → 隐私与安全性 → 辅助功能」中授予权限',
    text: '',
  };

  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    onMounted: () => {}, onUnmounted: () => {},
    t: (key) => key,
    invokeIpc: async (cmd) => {
      if (cmd === 'translator_get_config') {
        return { default_provider_id: 'google', target_lang: 'zh-CN', source_lang: 'auto' };
      }
      if (cmd === 'translator_list_providers') {
        return [
          { id: 'google', name: 'Google Translate', requires_api_key: true, has_api_key: providerHasKey },
          { id: 'deepl', name: 'DeepL', requires_api_key: true, has_api_key: false },
        ];
      }
      if (cmd === 'translator_take_selection') {
        return mockSelection;
      }
      return null;
    },
    console,
  });

  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  const run = (expression) => vm.runInContext(expression, context);

  // 初始化加载配置：发现当前 Google 未配置 Key
  await run('loadConfig()');
  assert.equal(run('missingApiKey.value'), true);

  // 消费选区：辅助功能未授权
  await run('consumeSelection()');
  assert.equal(run('permissionDenied.value'), true);
  assert.equal(run('permissionGuidance.value'), '请在「系统设置 → 隐私与安全性 → 辅助功能」中授予权限');
  // 关键断言：权限拒绝与缺少 Key 同时并存，缺少 Key 的提示与入口不被覆盖
  assert.equal(run('missingApiKey.value'), true);

  // 用户在控制台配置 Key 并返回气泡（触发聚焦重载）
  providerHasKey = true;
  await run('handleWindowFocus()');
  assert.equal(run('missingApiKey.value'), false);
});

test('Dashboard: applyDashboardRoute navigates to /translator tab and queues pending route', async () => {
  const source = await readFile(new URL('../../src/components/Dashboard.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/import\s+[\s\S]*?from\s+['"][^'"]+['"];?/g, '')
    .replace(/^import\s+[^;]+?;\s*/gm, '');

  let currentStatus = {
    active_window_count: 1,
    module_catalog: [
      { id: 'translator', enabled: true, supported: true },
      { id: 'media_proc', enabled: true, supported: true },
      { id: 'password_vault', enabled: true, supported: true },
      { id: 'screenshot', enabled: true, supported: true },
    ],
    config: { general: { language: 'zh-CN' } },
  };

  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    onMounted: () => {}, onUnmounted: () => {},
    t: (key) => key,
    invokeIpc: async (cmd) => {
      if (cmd === 'kernel_get_status') return currentStatus;
      return [];
    },
    syncLocaleFromBackendConfig: () => {},
    LOCALE_OPTIONS: [],
    ACCENT_PALETTE: [],
    currentLocale: { value: 'zh-CN' },
    currentAccent: { value: 'blue' },
    currentThemeMode: { value: 'light' },
    isDarkActive: { value: false },
    console,
  });

  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  const run = (expression) => vm.runInContext(expression, context);

  // 1. 刷新状态前请求 /translator 路由：暂存至 pendingRoute
  run("applyDashboardRoute('/translator')");
  assert.equal(run('currentTab.value'), 'overview');
  assert.equal(run('pendingRoute'), 'translator');

  // 2. 刷新后模块就绪，自动消费 pendingRoute 切换至 translator
  await run('refreshDashboard()');
  assert.equal(run('currentTab.value'), 'translator');
  assert.equal(run('pendingRoute'), null);

  // 3. 直接通过各种契约格式切换
  run("applyDashboardRoute('dashboard/translator')");
  assert.equal(run('currentTab.value'), 'translator');

  run("applyDashboardRoute('translator')");
  assert.equal(run('currentTab.value'), 'translator');

  run("applyDashboardRoute('/overview')");
  assert.equal(run('currentTab.value'), 'overview');

  run("applyDashboardRoute('/translator')");
  assert.equal(run('currentTab.value'), 'translator');
});

test('TranslateTrigger: never sends selection text and ignores repeated clicks while opening', async () => {
  const source = await readFile(new URL('../../src/components/TranslateTrigger.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/^import .*;$/gm, '');
  const calls = [];
  let finish;
  const context = vm.createContext({
    ref: (value) => ({ value }), t: (key) => key,
    invokeIpc: (...args) => { calls.push(args); return new Promise((resolve) => { finish = resolve; }); },
  });
  vm.runInContext(ts.transpileModule(script, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText, context);
  const run = (expression) => vm.runInContext(expression, context);
  assert.equal(calls.length, 0);
  const first = run('open()');
  await run('open()');
  assert.deepEqual(calls, [['translator_open_mouse_selection']]);
  finish({ opened: true });
  await first;
  assert.equal(run('opening.value'), false);
});
