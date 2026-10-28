import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';

async function loadSticky(options = {}) {
  const source = await readFile(new URL('../../src/components/StickyNote.vue', import.meta.url), 'utf8');
  const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script, 'must load the actual sticky note script');
  const parsed = ts.createSourceFile('StickyNote.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const body = ts.factory.updateSourceFile(parsed, parsed.statements.filter((node) => !ts.isImportDeclaration(node)));
  const code = ts.transpileModule(ts.createPrinter().printFile(body), {
    compilerOptions: { target: ts.ScriptTarget.ES2020 },
  }).outputText;

  const calls = [];
  const errors = [];
  const timers = new Map();
  const pending = [];
  const listeners = [];
  const hooks = { mounted: [], unmounted: [] };
  let nextTimer = 0;
  let destroyed = false;
  const fail = options.fail ?? new Set();
  const defer = options.defer ?? new Set();
  const context = vm.createContext({
    ref: (value) => ({ value }),
    computed: (get) => ({ get value() { return get(); } }),
    defineProps: () => ({ noteId: options.noteId || '' }),
    onMounted(fn) { hooks.mounted.push(fn); },
    onUnmounted(fn) { hooks.unmounted.push(fn); },
    t: (key) => key,
    URLSearchParams,
    window: options.window ?? {
      location: { search: options.search || '' },
      __TAURI__: {
        event: {
          listen(name, callback) {
            listeners.push({ name, callback });
            return Promise.resolve(() => {});
          },
        },
      },
    },
    setTimeout(fn, delay) {
      assert.equal(delay, 400, 'content debounce stays 400 ms');
      const id = ++nextTimer;
      timers.set(id, fn);
      return id;
    },
    clearTimeout(id) { timers.delete(id); },
    invokeIpc: async (name, payload = {}) => {
      calls.push({
        name,
        id: payload.id,
        title: payload.title,
        content: payload.content_md,
        theme: payload.theme_color,
        opacity: payload.opacity,
        isPinned: payload.is_pinned,
        writeSeq: payload.write_seq,
        sessionId: payload.session_id,
      });
      if (name === 'sticky_close_note') {
        destroyed = true;
        timers.clear();
      }
      if (fail.has(name)) throw new Error(`forced failure ${name}`);
      if (defer.has(name)) {
        return new Promise((resolve, reject) => pending.push({ name, payload, resolve, reject }));
      }
      if (name === 'sticky_begin_session') {
        const configured = options.sessionId;
        const sessionId = typeof configured === 'function' ? configured() : (configured ?? 1);
        return { id: payload.id, session_id: sessionId };
      }
      if (name === 'sticky_get_note') {
        return {
          id: payload.id,
          title: '服务器标题',
          content_md: '服务器正文',
          theme_color: 'amber',
          opacity: 0.95,
          pos_x: 10,
          pos_y: 20,
          width: 320,
          height: 300,
          is_pinned: true,
          is_open: true,
          updated_at: 1,
        };
      }
      return {
        id: payload.id,
        title: payload.title,
        content_md: payload.content_md,
        theme_color: payload.theme_color,
        opacity: payload.opacity,
        is_pinned: payload.is_pinned,
      };
    },
    console: { error: (...args) => errors.push(args.map(String).join(' ')) },
  });
  vm.runInContext(code, context);
  const run = (expression) => vm.runInContext(expression, context);
  if (options.note) {
    run(`note.value.id = ${JSON.stringify(options.note.id)};
      note.value.title = ${JSON.stringify(options.note.title)};
      note.value.content_md = ${JSON.stringify(options.note.content)};`);
  }

  async function until(predicate) {
    for (let attempt = 0; attempt < 40; attempt += 1) {
      if (predicate()) return;
      await Promise.resolve();
    }
    assert.fail(`timed out; calls=${JSON.stringify(calls.map((call) => call.name))}`);
  }

  return {
    calls,
    errors,
    listeners,
    hooks,
    get destroyed() { return destroyed; },
    get pendingTimers() { return timers.size; },
    get pendingCount() { return pending.length; },
    run,
    scheduleSave: () => run('saveContentDebounced()'),
    close: () => run('closeNoteWindow()'),
    setTheme: (theme) => run(`setTheme(${JSON.stringify(theme)})`),
    togglePin: () => run('togglePin()'),
    setOpacityPercent: (percent) => run(`onOpacityChange({ target: { value: ${JSON.stringify(String(percent))} } })`),
    async flushTimer() {
      const next = timers.entries().next().value;
      assert.ok(next, 'expected a pending debounce timer');
      const [id, callback] = next;
      timers.delete(id);
      await callback();
    },
    resolveHead(value) {
      const item = pending.shift();
      assert.ok(item, 'no deferred IPC to resolve');
      item.resolve(value ?? {
        id: item.payload.id,
        title: item.payload.title,
        content_md: item.payload.content_md,
        theme_color: item.payload.theme_color,
        opacity: item.payload.opacity,
        is_pinned: item.payload.is_pinned,
      });
    },
    rejectHead(error = new Error('save failed')) {
      const item = pending.shift();
      assert.ok(item, 'no deferred IPC to reject');
      item.reject(error);
    },
    until,
  };
}

const qaNote = { id: 'qa-note', title: 'QA fictional title', content: 'QA fictional content' };

test('ordinary debounce saves the fictional note when its timer fires', async () => {
  const sticky = await loadSticky({ note: qaNote });
  sticky.scheduleSave();
  assert.equal(sticky.pendingTimers, 1);
  await sticky.flushTimer();
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content']);
  assert.equal(sticky.calls[0].title, qaNote.title);
  assert.equal(sticky.calls[0].content, qaNote.content);
  assert.equal(sticky.calls[0].writeSeq, 1);
});

test('closing while a save is pending persists edits before destroying the WebView', async () => {
  const sticky = await loadSticky({ note: qaNote });
  sticky.scheduleSave();
  assert.equal(sticky.pendingTimers, 1, 'the edit must be pending before close');
  await sticky.close();
  assert.equal(sticky.destroyed, true);
  assert.equal(sticky.pendingTimers, 0);
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content', 'sticky_close_note']);
  assert.equal(sticky.calls[0].title, qaNote.title);
  assert.equal(sticky.calls[0].content, qaNote.content);
});

test('close waits for an unfinished save before destroying the WebView', async () => {
  const sticky = await loadSticky({
    note: qaNote,
    defer: new Set(['sticky_save_content']),
  });
  sticky.scheduleSave();
  await sticky.flushTimer();
  assert.equal(sticky.calls.length, 1);
  const closing = sticky.close();
  await Promise.resolve();
  assert.equal(sticky.destroyed, false);
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content']);
  sticky.resolveHead({ title: qaNote.title, content_md: qaNote.content });
  await closing;
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content', 'sticky_close_note']);
  assert.equal(sticky.calls[0].content, qaNote.content);
});

test('a save generated while close is waiting is persisted before destroy', async () => {
  const sticky = await loadSticky({
    note: { ...qaNote, content: 'version one' },
    defer: new Set(['sticky_save_content']),
  });
  sticky.scheduleSave();
  await sticky.flushTimer();
  const closing = sticky.close();
  await Promise.resolve();
  sticky.run('note.value.title = "generated title"');
  sticky.run('note.value.content_md = "generated body"');
  sticky.scheduleSave();
  assert.equal(sticky.destroyed, false);
  sticky.resolveHead({ title: qaNote.title, content_md: 'version one' });
  await sticky.until(() => sticky.calls.filter((call) => call.name === 'sticky_save_content').length === 2);
  assert.equal(sticky.calls[1].title, 'generated title');
  assert.equal(sticky.calls[1].content, 'generated body');
  assert.equal(sticky.destroyed, false);
  sticky.resolveHead({ title: 'generated title', content_md: 'generated body' });
  await closing;
  assert.equal(sticky.calls.at(-1).name, 'sticky_close_note');
  assert.deepEqual(
    sticky.calls.filter((call) => call.name === 'sticky_save_content').map((call) => call.content),
    ['version one', 'generated body'],
  );
});

test('closing without pending edits only closes', async () => {
  const sticky = await loadSticky({ note: qaNote });
  await sticky.close();
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_close_note']);
});

test('a failed save keeps the note open and a retry then persists the same text', async () => {
  const fail = new Set(['sticky_save_content']);
  const sticky = await loadSticky({ note: qaNote, fail });
  sticky.scheduleSave();
  await sticky.close();
  assert.equal(sticky.destroyed, false);
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content']);
  assert.equal(sticky.run('note.value.content_md'), qaNote.content);
  assert.equal(sticky.run('note.value.title'), qaNote.title);
  assert.equal(sticky.run('saveError.value'), true);
  assert.match(sticky.errors.join('\n'), /关闭前保存便笺失败/);

  fail.delete('sticky_save_content');
  await sticky.close();
  assert.equal(sticky.destroyed, true);
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content', 'sticky_save_content', 'sticky_close_note']);
  assert.equal(sticky.calls[1].content, qaNote.content);
  assert.ok(sticky.calls[1].writeSeq > sticky.calls[0].writeSeq);
});

test('close waits for an in-flight save and persists newer text, theme, and window state', async () => {
  const sticky = await loadSticky({
    note: { ...qaNote, content: 'version one' },
    defer: new Set(['sticky_save_content', 'sticky_update_theme', 'sticky_update_window_state']),
  });
  sticky.scheduleSave();
  await sticky.flushTimer();
  sticky.run('note.value.title = "QA newer title"');
  sticky.run('note.value.content_md = "version two"');
  sticky.scheduleSave();
  sticky.setTheme('rose');
  sticky.setOpacityPercent(80);
  sticky.togglePin();
  const closing = sticky.close();

  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_save_content']);
  assert.equal(sticky.calls[0].content, 'version one');
  assert.equal(sticky.destroyed, false);

  sticky.resolveHead({ title: qaNote.title, content_md: 'version one' });
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_update_theme'));
  assert.equal(sticky.calls[1].theme, 'rose');
  sticky.resolveHead({ theme_color: 'rose' });

  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_update_window_state'));
  const latestWindow = sticky.calls.find((call) => call.name === 'sticky_update_window_state');
  assert.equal(latestWindow.opacity, 0.8);
  assert.equal(latestWindow.isPinned, false);
  assert.equal(sticky.destroyed, false);
  sticky.resolveHead({ opacity: 0.8, is_pinned: false });

  await sticky.until(() => sticky.calls.filter((call) => call.name === 'sticky_save_content').length === 2);
  const latestContent = sticky.calls.filter((call) => call.name === 'sticky_save_content').at(-1);
  assert.equal(latestContent.title, 'QA newer title');
  assert.equal(latestContent.content, 'version two');
  assert.ok(latestContent.writeSeq > sticky.calls[0].writeSeq);
  sticky.resolveHead({ title: 'QA newer title', content_md: 'version two' });
  await closing;

  assert.equal(sticky.calls.at(-1).name, 'sticky_close_note');
  assert.equal(sticky.destroyed, true);
  const contentWrites = sticky.calls.filter((call) => call.name === 'sticky_save_content').map((call) => call.content);
  assert.deepEqual(contentWrites, ['version one', 'version two']);
});

test('a rejected in-flight save does not destroy the note and the retry stores the edited text', async () => {
  const sticky = await loadSticky({
    note: qaNote,
    defer: new Set(['sticky_save_content']),
  });
  sticky.scheduleSave();
  await sticky.flushTimer();
  sticky.run('note.value.title = "kept title"');
  sticky.run('note.value.content_md = "kept body"');
  sticky.scheduleSave();
  const closing = sticky.close();
  sticky.rejectHead();
  await sticky.until(() => sticky.calls.filter((call) => call.name === 'sticky_save_content').length === 2);
  assert.equal(sticky.calls[1].title, 'kept title');
  assert.equal(sticky.calls[1].content, 'kept body');
  sticky.rejectHead();
  await closing;
  assert.equal(sticky.destroyed, false);
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_close_note'), false);
  assert.equal(sticky.run('note.value.content_md'), 'kept body');
  assert.equal(sticky.run('saveError.value'), true);
});

test('a slow older window update cannot outrun the latest opacity and pin before close', async () => {
  const sticky = await loadSticky({
    note: qaNote,
    defer: new Set(['sticky_update_window_state']),
  });
  sticky.setOpacityPercent(40);
  await sticky.until(() => sticky.calls.length >= 1);
  assert.equal(sticky.calls[0].opacity, 0.4);
  sticky.setOpacityPercent(80);
  sticky.togglePin();
  const closing = sticky.close();
  assert.equal(sticky.calls[0].isPinned, true);
  assert.equal(sticky.destroyed, false);
  sticky.resolveHead({ opacity: 0.4, is_pinned: true });
  await sticky.until(() => sticky.calls.some((call) => call.opacity === 0.8));
  const latest = sticky.calls.filter((call) => call.name === 'sticky_update_window_state').at(-1);
  assert.equal(latest.opacity, 0.8);
  assert.equal(latest.isPinned, false);
  assert.ok(latest.writeSeq > sticky.calls[0].writeSeq);
  sticky.resolveHead({ opacity: 0.8, is_pinned: false });
  await closing;
  assert.equal(sticky.calls.at(-1).name, 'sticky_close_note');
  assert.equal(sticky.calls.filter((call) => call.opacity === 0.4).length, 1);
});

test('theme save failure blocks close until a later attempt succeeds', async () => {
  const fail = new Set(['sticky_update_theme']);
  const sticky = await loadSticky({ note: qaNote, fail });
  sticky.setTheme('violet');
  await sticky.close();
  assert.equal(sticky.destroyed, false);
  assert.deepEqual(sticky.calls.map((call) => call.name), ['sticky_update_theme', 'sticky_update_theme']);
  assert.equal(sticky.run('note.value.theme_color'), 'violet');
  assert.match(sticky.errors.join('\n'), /关闭前保存便笺失败/);

  fail.delete('sticky_update_theme');
  await sticky.close();
  assert.deepEqual(
    sticky.calls.map((call) => call.name),
    ['sticky_update_theme', 'sticky_update_theme', 'sticky_update_theme', 'sticky_close_note'],
  );
  assert.equal(sticky.calls[2].theme, 'violet');
  assert.equal(sticky.destroyed, true);
});

test('initial load keeps typed fields and does not save placeholders before the note id exists', async () => {
  const sticky = await loadSticky({
    noteId: 'qa-note',
    defer: new Set(['sticky_get_note']),
  });
  const mounted = sticky.hooks.mounted[0]();
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_get_note'));
  sticky.run('note.value.title = "typed title"');
  sticky.run('note.value.content_md = "typed body"');
  sticky.scheduleSave();
  await sticky.close();
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_save_content'), false);
  assert.equal(sticky.destroyed, false);

  sticky.resolveHead({
    id: 'qa-note',
    title: '服务器标题',
    content_md: '服务器正文',
    theme_color: 'sky',
    opacity: 0.4,
    pos_x: 10,
    pos_y: 20,
    width: 320,
    height: 300,
    is_pinned: true,
    is_open: true,
    updated_at: 1,
  });
  await mounted;
  assert.equal(sticky.run('note.value.title'), 'typed title');
  assert.equal(sticky.run('note.value.content_md'), 'typed body');
  assert.equal(sticky.run('note.value.theme_color'), 'sky');
  await sticky.close();
  const save = sticky.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(save.title, 'typed title');
  assert.equal(save.content, 'typed body');
  assert.equal(sticky.calls.at(-1).name, 'sticky_close_note');
});

test('typing while the write session is still starting survives the later note load', async () => {
  const sticky = await loadSticky({
    noteId: 'qa-note',
    defer: new Set(['sticky_begin_session', 'sticky_get_note']),
    sessionId: 3,
  });
  const mounted = sticky.hooks.mounted[0]();
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_begin_session'));
  sticky.run('note.value.title = "会话期间标题"');
  sticky.run('note.value.content_md = "会话期间正文"');
  sticky.run('note.value.theme_color = "rose"');
  sticky.run('note.value.opacity = 0.4');
  sticky.run('note.value.is_pinned = false');
  sticky.scheduleSave();
  sticky.resolveHead({ id: 'qa-note', session_id: 3 });
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_get_note'));
  sticky.resolveHead({
    id: 'qa-note',
    title: '服务器标题',
    content_md: '服务器正文',
    theme_color: 'sky',
    opacity: 0.95,
    pos_x: 10,
    pos_y: 20,
    width: 320,
    height: 300,
    is_pinned: true,
    is_open: true,
    updated_at: 1,
  });
  await mounted;
  assert.equal(sticky.run('note.value.title'), '会话期间标题');
  assert.equal(sticky.run('note.value.content_md'), '会话期间正文');
  assert.equal(sticky.run('note.value.theme_color'), 'rose');
  assert.equal(sticky.run('note.value.opacity'), 0.4);
  assert.equal(sticky.run('note.value.is_pinned'), false);
});

test('a title edit during load keeps the untouched server body', async () => {
  const sticky = await loadSticky({
    noteId: 'qa-note',
    defer: new Set(['sticky_get_note']),
  });
  const mounted = sticky.hooks.mounted[0]();
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_get_note'));
  sticky.run('note.value.title = "typed title"');
  sticky.scheduleSave();
  sticky.resolveHead({
    id: 'qa-note',
    title: '服务器标题',
    content_md: '服务器正文',
    theme_color: 'sky',
    opacity: 0.95,
    pos_x: 10,
    pos_y: 20,
    width: 320,
    height: 300,
    is_pinned: true,
    is_open: true,
    updated_at: 1,
  });
  await mounted;
  assert.equal(sticky.run('note.value.title'), 'typed title');
  assert.equal(sticky.run('note.value.content_md'), '服务器正文');
  await sticky.close();
  const save = sticky.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(save.title, 'typed title');
  assert.equal(save.content, '服务器正文');
});

test('native or library flush-close saves pending edits before destroying the note', async () => {
  const sticky = await loadSticky({ noteId: 'qa-note' });
  await sticky.hooks.mounted[0]();
  assert.equal(sticky.listeners[0].name, 'sticky://flush-close');
  sticky.run('note.value.title = "flush title"');
  sticky.run('note.value.content_md = "flush body"');
  sticky.scheduleSave();
  sticky.listeners[0].callback({ payload: { id: 'qa-note' } });
  await sticky.until(() => sticky.destroyed);
  assert.deepEqual(
    sticky.calls
      .filter((call) => call.name !== 'sticky_get_note' && call.name !== 'sticky_begin_session')
      .map((call) => call.name),
    ['sticky_save_content', 'sticky_close_note'],
  );
  assert.equal(sticky.calls.find((call) => call.name === 'sticky_save_content').content, 'flush body');
});

test('a reopened note begins a new write session and restarts title, body, theme, and window sequences', async () => {
  const first = await loadSticky({ noteId: 'qa-note', sessionId: 4 });
  await first.hooks.mounted[0]();
  await first.until(() => first.calls.some((call) => call.name === 'sticky_begin_session'));
  first.run('note.value.title = "第一轮标题"');
  first.run('note.value.content_md = "第一轮正文"');
  first.scheduleSave();
  await first.flushTimer();
  first.setTheme('rose');
  first.togglePin();
  await first.until(() => first.calls.some((call) => call.name === 'sticky_update_window_state'));
  const firstSave = first.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(firstSave.writeSeq, 1);
  assert.equal(firstSave.sessionId, 4);
  assert.equal(first.calls.find((call) => call.name === 'sticky_update_theme').sessionId, 4);
  assert.equal(first.calls.find((call) => call.name === 'sticky_update_window_state').sessionId, 4);
  await first.close();

  const second = await loadSticky({ noteId: 'qa-note', sessionId: 11 });
  await second.hooks.mounted[0]();
  await second.until(() => second.calls.some((call) => call.name === 'sticky_begin_session'));
  second.run('note.value.title = "第二轮标题"');
  second.run('note.value.content_md = "第二轮正文"');
  second.scheduleSave();
  await second.flushTimer();
  second.setTheme('violet');
  second.setOpacityPercent(80);
  await second.until(() => second.calls.some((call) => call.name === 'sticky_update_window_state'));

  const save = second.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(save.title, '第二轮标题');
  assert.equal(save.content, '第二轮正文');
  assert.equal(save.writeSeq, 1);
  assert.equal(save.sessionId, 11);
  const theme = second.calls.find((call) => call.name === 'sticky_update_theme');
  assert.equal(theme.theme, 'violet');
  assert.equal(theme.writeSeq, 1);
  assert.equal(theme.sessionId, 11);
  const windowCall = second.calls.find((call) => call.name === 'sticky_update_window_state');
  assert.equal(windowCall.opacity, 0.8);
  assert.equal(windowCall.writeSeq, 1);
  assert.equal(windowCall.sessionId, 11);
});

function writesOf(sticky) {
  return sticky.calls.filter((call) => (
    call.name === 'sticky_save_content'
    || call.name === 'sticky_update_theme'
    || call.name === 'sticky_update_window_state'
  ));
}

test('a failed write session keeps title, body, theme, and window edits unsaved and open', async () => {
  const fail = new Set(['sticky_begin_session']);
  const sticky = await loadSticky({ noteId: 'qa-note', fail });
  await sticky.hooks.mounted[0]();
  sticky.run('note.value.title = "未保存标题"');
  sticky.run('note.value.content_md = "未保存正文"');
  sticky.scheduleSave();
  sticky.setTheme('rose');
  sticky.togglePin();
  await sticky.close();
  assert.equal(sticky.destroyed, false);
  assert.equal(sticky.run('saveError.value'), true);
  assert.equal(sticky.run('note.value.title'), '未保存标题');
  assert.equal(sticky.run('note.value.content_md'), '未保存正文');
  assert.equal(sticky.run('note.value.theme_color'), 'rose');
  assert.equal(sticky.run('note.value.is_pinned'), false);
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_close_note'), false);
  assert.equal(writesOf(sticky).length, 0);
  assert.match(sticky.errors.join('\n'), /开始便笺写入会话失败/);

  fail.delete('sticky_begin_session');
  await sticky.close();
  assert.equal(sticky.destroyed, true);
  const save = sticky.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(save.title, '未保存标题');
  assert.equal(save.content, '未保存正文');
  assert.equal(save.sessionId, 1);
  const theme = sticky.calls.find((call) => call.name === 'sticky_update_theme');
  assert.equal(theme.theme, 'rose');
  assert.equal(theme.sessionId, 1);
  const windowCall = sticky.calls.find((call) => call.name === 'sticky_update_window_state');
  assert.equal(windowCall.isPinned, false);
  assert.equal(windowCall.sessionId, 1);
});

test('an invalid write session id keeps pending edits unsaved and does not close', async () => {
  let sessionId = 0;
  const sticky = await loadSticky({
    noteId: 'qa-note',
    sessionId: () => sessionId,
  });
  await sticky.hooks.mounted[0]();
  sticky.run('note.value.title = "无效会话标题"');
  sticky.run('note.value.content_md = "无效会话正文"');
  sticky.scheduleSave();
  sticky.setTheme('violet');
  sticky.setOpacityPercent(40);
  await sticky.close();
  assert.equal(sticky.destroyed, false);
  assert.equal(sticky.run('saveError.value'), true);
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_close_note'), false);
  assert.equal(writesOf(sticky).length, 0);
  assert.match(sticky.errors.join('\n'), /便笺写入会话无效/);

  sessionId = 6;
  await sticky.close();
  assert.equal(sticky.destroyed, true);
  const save = sticky.calls.find((call) => call.name === 'sticky_save_content');
  assert.equal(save.title, '无效会话标题');
  assert.equal(save.content, '无效会话正文');
  assert.equal(save.sessionId, 6);
  assert.equal(sticky.calls.find((call) => call.name === 'sticky_update_theme').sessionId, 6);
  assert.equal(sticky.calls.find((call) => call.name === 'sticky_update_window_state').sessionId, 6);
  assert.equal(sticky.calls.find((call) => call.name === 'sticky_update_window_state').opacity, 0.4);
});

test('a rejected content write is not treated as saved when the text matches', async () => {
  const sticky = await loadSticky({
    note: qaNote,
    defer: new Set(['sticky_save_content']),
  });
  sticky.scheduleSave();
  const closing = sticky.close();
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_save_content'));
  sticky.resolveHead({
    applied: false,
    title: qaNote.title,
    content_md: qaNote.content,
  });
  await closing;
  assert.equal(sticky.destroyed, false);
  assert.equal(sticky.run('saveError.value'), true);
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_close_note'), false);
  assert.match(sticky.errors.join('\n'), /便笺写入已被拒绝/);
});

test('a rejected theme or window write stays pending and blocks close', async () => {
  const sticky = await loadSticky({
    note: qaNote,
    defer: new Set(['sticky_update_theme', 'sticky_update_window_state']),
  });
  sticky.setTheme('rose');
  sticky.togglePin();
  const closing = sticky.close();
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_update_theme'));
  sticky.resolveHead({ applied: false, theme_color: 'amber' });
  await sticky.until(() => sticky.calls.some((call) => call.name === 'sticky_update_window_state'));
  sticky.resolveHead({ applied: false, opacity: 0.95, is_pinned: true });
  await sticky.until(() => sticky.calls.filter((call) => call.name === 'sticky_update_theme').length === 2);
  sticky.resolveHead({ applied: false, theme_color: 'amber' });
  await closing;
  assert.equal(sticky.destroyed, false);
  assert.equal(sticky.run('saveError.value'), true);
  assert.equal(sticky.run('note.value.theme_color'), 'rose');
  assert.equal(sticky.run('note.value.is_pinned'), false);
  assert.equal(sticky.calls.some((call) => call.name === 'sticky_close_note'), false);
  assert.match(sticky.errors.join('\n'), /便笺写入已被拒绝/);
});
