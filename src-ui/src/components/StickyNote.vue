<template>
  <div
    class="macos-sticky-frame"
    :class="`sticky-theme-${note.theme_color || 'neutral'}`"
    :style="{ opacity: note.opacity }"
  >
    <!-- 便笺标题栏 -->
    <header class="sticky-header">
      <div class="header-left">
        <input
          v-model="note.title"
          class="title-input"
          :placeholder="t('sticky.window.untitledPlaceholder')"
          @blur="saveContentDebounced"
        />
      </div>

      <!-- 右侧控制区 -->
      <div class="header-right">
        <!-- 切换视图：编辑 / 预览 -->
        <button
          class="tool-btn"
          :class="{ active: viewMode === 'preview' }"
          @click="toggleViewMode"
          :title="viewMode === 'edit' ? t('sticky.window.switchPreviewTooltip') : t('sticky.window.switchEditTooltip')"
        >
          <Icons :name="viewMode === 'edit' ? 'text' : 'check'" size="12" />
          <span>{{ viewMode === 'edit' ? t('sticky.window.previewBtn') : t('sticky.window.editBtn') }}</span>
        </button>

        <!-- 置顶开关 -->
        <button
          class="tool-btn"
          :class="{ active: note.is_pinned }"
          @click="togglePin"
          :title="t('sticky.window.pinTooltip')"
        >
          <Icons :name="note.is_pinned ? 'pin-fill' : 'pin'" size="12" />
        </button>

        <!-- 调色盘弹出切换 -->
        <div class="palette-dropdown-wrap">
          <button class="tool-btn" @click="showPalette = !showPalette" :title="t('sticky.window.paletteTooltip')">
            <Icons name="palette" size="12" />
          </button>
          <div v-if="showPalette" class="palette-popover">
            <button
              v-for="color in themeOptions"
              :key="color.key"
              class="palette-dot"
              :style="{ background: color.color }"
              :title="t(`sticky.window.themes.${color.key}`)"
              @click="setTheme(color.key)"
            ></button>
          </div>
        </div>

        <!-- 导出 Markdown -->
        <button class="tool-btn" @click="exportMarkdownFile" :title="t('sticky.window.exportMdTooltip')">
          <Icons name="download" size="12" />
        </button>

        <!-- 关闭并销毁原生窗口 -->
        <button class="tool-btn close-btn" @click="closeNoteWindow" :title="t('sticky.window.closeTooltip')">
          <Icons name="close" size="12" />
        </button>
      </div>
    </header>

    <!-- 便笺编辑与排版主区域 -->
    <main class="sticky-body">
      <!-- 模式一：Markdown 源码编辑输入区 -->
      <textarea
        v-if="viewMode === 'edit'"
        ref="editorRef"
        v-model="note.content_md"
        class="sticky-textarea"
        :placeholder="t('sticky.window.editorPlaceholder')"
        @input="saveContentDebounced"
      ></textarea>

      <!-- 模式二：KaTeX + Markdown 实时排版渲染区 -->
      <div
        v-else
        class="sticky-rendered-content"
        v-html="renderedHtml"
        @click="handleContentClick"
      ></div>
    </main>

    <!-- 便笺底栏：透明度滑块与存盘状态 -->
    <footer class="sticky-footer">
      <div class="opacity-slider-wrap">
        <span class="opacity-label">{{ t('sticky.window.opacityLabel', { percent: Math.round(note.opacity * 100) }) }}</span>
        <input
          type="range"
          min="35"
          max="100"
          :value="Math.round(note.opacity * 100)"
          class="opacity-range"
          @input="onOpacityChange"
        />
      </div>

      <div class="save-status">
        <span v-if="saving">{{ t('sticky.window.saving') }}</span>
        <span v-else-if="saveError">{{ t('sticky.window.saveFailed') }}</span>
        <span v-else-if="lastSavedTime">{{ t('sticky.window.savedAt', { time: lastSavedTime }) }}</span>
      </div>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { marked } from 'marked';
import katex from 'katex';
import 'katex/dist/katex.min.css';
import { invokeIpc, type StickyNoteRecord } from '../api/ipc';
import Icons from './Icons.vue';
import { t } from '../utils/i18n';

const props = defineProps<{
  noteId?: string;
}>();

/** 响应式便笺数据 */
const note = ref<StickyNoteRecord>({
  id: '',
  title: '便笺',
  content_md: '',
  theme_color: 'neutral',
  opacity: 0.95,
  pos_x: null,
  pos_y: null,
  width: 320,
  height: 300,
  is_pinned: true,
  is_open: true,
  updated_at: Date.now(),
});

/** 视图模式：'edit' (编辑源码) | 'preview' (排版渲染) */
const viewMode = ref<'edit' | 'preview'>('edit');
/** 调色盘弹层开关 */
const showPalette = ref(false);
/** 正在存盘标记 */
const saving = ref(false);
/** 最近一次待保存写入失败。关闭会被取消，窗口保持打开。 */
const saveError = ref(false);
/** 上次存盘时间 */
const lastSavedTime = ref('');
/** 编辑器输入引用 */
const editorRef = ref<HTMLTextAreaElement | null>(null);

/** 主题色彩列表 */
const themeOptions = [
  { key: 'neutral', color: '#2c2c2e' },
  { key: 'amber', color: '#451a03' },
  { key: 'sky', color: '#082f49' },
  { key: 'emerald', color: '#064e3b' },
  { key: 'rose', color: '#4c0519' },
  { key: 'violet', color: '#2e1065' },
];

/**
 * 核心渲染器：将文本中的 Markdown 语法与 LaTeX 数学公式渲染为标准 HTML
 */
const renderedHtml = computed(() => {
  let content = note.value.content_md || '';
  if (!content.trim()) {
    return `<p style="color:#6e6e73;font-style:italic">${t('sticky.window.emptyRenderedHint')}</p>`;
  }

  // 1. 优先渲染块级数学公式 $$...$$
  content = content.replace(/\$\$([\s\S]+?)\$\$/g, (_, mathExpr) => {
    try {
      return `<div class="katex-block-wrap">${katex.renderToString(mathExpr.trim(), {
        displayMode: true,
        throwOnError: false,
      })}</div>`;
    } catch {
      return `<div class="katex-error">${mathExpr}</div>`;
    }
  });

  // 2. 渲染行内数学公式 $...$
  content = content.replace(/\$([^\$\n]+?)\$/g, (_, mathExpr) => {
    try {
      return katex.renderToString(mathExpr.trim(), {
        displayMode: false,
        throwOnError: false,
      });
    } catch {
      return `$${mathExpr}$`;
    }
  });

  // 3. 执行标准 Markdown 解析 (标题、待办清单、加粗等)
  try {
    return marked.parse(content, { breaks: true, gfm: true }) as string;
  } catch {
    return content;
  }
});

/** 标题与正文防抖。关闭时如果这个计时器还在，写入请求尚未发出。 */
const CONTENT_SAVE_DELAY_MS = 400;
let debounceTimer: any = null;
/** 同一便笺的保存串行执行，避免较慢的旧请求覆盖更新的标题、正文或窗口状态。 */
let writeTail: Promise<void> = Promise.resolve();
let closing = false;
let contentDirty = false;
let themeDirty = false;
let windowDirty = false;
/** 本次 WebView 内的发送序号。重新打开后从 0 计，靠写入会话和旧页面区分。 */
let nextContentSeq = 0;
let nextThemeSeq = 0;
let nextWindowSeq = 0;
let writeSessionId: number | null = null;
let sessionReady: Promise<void> = Promise.resolve();
let sessionPending = false;
let sessionRequired = false;
let sessionNoteId = '';
let unlistenFlush: (() => void) | undefined;

function startWriteSession(noteId: string) {
  if (!noteId) return sessionReady;
  sessionRequired = true;
  sessionNoteId = noteId;
  sessionPending = true;
  writeSessionId = null;
  sessionReady = invokeIpc<{ session_id?: number }>('sticky_begin_session', { id: noteId })
    .then((res) => {
      const sessionId = Number(res?.session_id);
      if (!Number.isFinite(sessionId) || sessionId <= 0) {
        throw new Error('便笺写入会话无效');
      }
      writeSessionId = sessionId;
    })
    .finally(() => {
      sessionPending = false;
    });
  return sessionReady;
}

/** 只有显式开过会话时才等待。未开会话的旧调用不插入额外异步节拍。 */
function prepareSession(): Promise<void> | null {
  if (!sessionRequired || writeSessionId != null) return null;
  return (async () => {
    if (sessionPending) {
      try {
        await sessionReady;
      } catch (err) {
        console.error('开始便笺写入会话失败:', err);
      }
    }
    if (writeSessionId == null) await startWriteSession(sessionNoteId || note.value.id);
  })();
}

function withSession<T extends Record<string, unknown>>(payload: T) {
  if (writeSessionId == null) return payload;
  return { ...payload, session_id: writeSessionId };
}

function assertApplied(saved: { applied?: boolean } | null | undefined) {
  if (saved?.applied === false) throw new Error('便笺写入已被拒绝');
}

function enqueue(task: () => Promise<void>): Promise<void> {
  const run = writeTail.then(task, task);
  writeTail = run.then(() => undefined, () => undefined);
  return run;
}

function formatSavedTime() {
  const now = new Date();
  const h = now.getHours().toString().padStart(2, '0');
  const m = now.getMinutes().toString().padStart(2, '0');
  const s = now.getSeconds().toString().padStart(2, '0');
  return `${h}:${m}:${s}`;
}

function clearSaveErrorIfClean() {
  if (!contentDirty && !themeDirty && !windowDirty) saveError.value = false;
}

function scheduleContentTimer() {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    void persistContent().catch((err) => {
      console.error('保存便笺失败:', err);
    });
  }, CONTENT_SAVE_DELAY_MS);
}

/** 记录最新标题与正文，并在 400ms 内没有继续输入时写入。 */
function saveContentDebounced() {
  contentDirty = true;
  saving.value = true;
  saveError.value = false;
  if (closing) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
    return;
  }
  scheduleContentTimer();
}

function persistContent() {
  return enqueue(async () => {
    if (!contentDirty) return;
    const pendingSession = prepareSession();
    if (pendingSession) {
      try {
        await pendingSession;
      } catch (err) {
        saving.value = false;
        saveError.value = true;
        throw err;
      }
    }
    if (!note.value.id) {
      saving.value = false;
      saveError.value = true;
      throw new Error('便笺尚未加载，无法保存');
    }
    const snapshot = {
      id: note.value.id,
      title: note.value.title,
      content: note.value.content_md,
    };
    const writeSeq = ++nextContentSeq;
    contentDirty = false;
    try {
      const saved = await invokeIpc<StickyNoteRecord & { applied?: boolean }>('sticky_save_content', withSession({
        id: snapshot.id,
        title: snapshot.title,
        content_md: snapshot.content,
        write_seq: writeSeq,
      }));
      assertApplied(saved);
      const localMoved = note.value.title !== snapshot.title || note.value.content_md !== snapshot.content;
      const storedMoved = !!saved
        && (saved.title !== snapshot.title || saved.content_md !== snapshot.content);
      if (localMoved || storedMoved) {
        contentDirty = true;
        saving.value = true;
        if (!closing && debounceTimer === null) scheduleContentTimer();
      } else {
        saving.value = false;
        lastSavedTime.value = formatSavedTime();
        clearSaveErrorIfClean();
      }
    } catch (err) {
      contentDirty = true;
      saving.value = false;
      saveError.value = true;
      throw err;
    }
  });
}

function persistTheme() {
  return enqueue(async () => {
    if (!themeDirty) return;
    const pendingSession = prepareSession();
    if (pendingSession) {
      try {
        await pendingSession;
      } catch (err) {
        saveError.value = true;
        throw err;
      }
    }
    if (!note.value.id) {
      saveError.value = true;
      throw new Error('便笺尚未加载，无法保存');
    }
    const snapshot = { id: note.value.id, theme: note.value.theme_color };
    const writeSeq = ++nextThemeSeq;
    themeDirty = false;
    try {
      const saved = await invokeIpc<{ applied?: boolean }>('sticky_update_theme', withSession({
        id: snapshot.id,
        theme_color: snapshot.theme,
        write_seq: writeSeq,
      }));
      assertApplied(saved);
      if (note.value.theme_color !== snapshot.theme) themeDirty = true;
      else clearSaveErrorIfClean();
    } catch (err) {
      themeDirty = true;
      saveError.value = true;
      throw err;
    }
  });
}

function persistWindow() {
  return enqueue(async () => {
    if (!windowDirty) return;
    const pendingSession = prepareSession();
    if (pendingSession) {
      try {
        await pendingSession;
      } catch (err) {
        saveError.value = true;
        throw err;
      }
    }
    if (!note.value.id) {
      saveError.value = true;
      throw new Error('便笺尚未加载，无法保存');
    }
    const snapshot = {
      id: note.value.id,
      opacity: note.value.opacity,
      isPinned: note.value.is_pinned,
    };
    const writeSeq = ++nextWindowSeq;
    windowDirty = false;
    try {
      const saved = await invokeIpc<{ applied?: boolean }>('sticky_update_window_state', withSession({
        id: snapshot.id,
        opacity: snapshot.opacity,
        is_pinned: snapshot.isPinned,
        write_seq: writeSeq,
      }));
      assertApplied(saved);
      if (note.value.opacity !== snapshot.opacity || note.value.is_pinned !== snapshot.isPinned) {
        windowDirty = true;
      } else {
        clearSaveErrorIfClean();
      }
    } catch (err) {
      windowDirty = true;
      saveError.value = true;
      throw err;
    }
  });
}

/** 等未完成的写入，以及等待期间新产生的标题、正文、主题和窗口状态保存。 */
async function flushDirtyState() {
  let waited: Promise<void> = writeTail;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    waited = writeTail;
    await waited;
    if (contentDirty || themeDirty || windowDirty) {
      await persistContent();
      await persistTheme();
      await persistWindow();
      continue;
    }
    if (writeTail !== waited) continue;
    return;
  }
  if (contentDirty || themeDirty || windowDirty || writeTail !== waited) {
    throw new Error('便笺仍有未保存的更改');
  }
}

/** 切换编辑/预览视图 */
function toggleViewMode() {
  viewMode.value = viewMode.value === 'edit' ? 'preview' : 'edit';
}

/** 切换置顶状态 */
function togglePin() {
  note.value.is_pinned = !note.value.is_pinned;
  windowDirty = true;
  void persistWindow().catch((err) => {
    console.error('更新置顶状态失败:', err);
  });
}

/** 调节透明度 */
function onOpacityChange(e: Event) {
  const val = Number((e.target as HTMLInputElement).value);
  note.value.opacity = Math.max(0.35, Math.min(1.0, val / 100.0));
  windowDirty = true;
  void persistWindow().catch((err) => {
    console.error('更新透明度失败:', err);
  });
}

/** 切换色彩主题 */
function setTheme(colorKey: string) {
  note.value.theme_color = colorKey;
  showPalette.value = false;
  themeDirty = true;
  void persistTheme().catch((err) => {
    console.error('更新主题色失败:', err);
  });
}

/** 导出便笺为 .md 文件 */
async function exportMarkdownFile() {
  try {
    const res = await invokeIpc<{ id: string; markdown: string }>('sticky_export_markdown', {
      id: note.value.id,
    });
    const blob = new Blob([res.markdown], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `${note.value.title || t('sticky.exportFilePrefix')}_${note.value.id.slice(0, 8)}.md`;
    link.click();
    URL.revokeObjectURL(url);
  } catch (err) {
    console.error('导出便笺失败:', err);
  }
}

/**
 * 关闭并销毁便笺 WebView。
 * 先等未完成的保存，再等关闭等待期间新产生的保存；失败时保留窗口和未保存内容。
 */
async function closeNoteWindow() {
  if (closing) return;
  closing = true;
  clearTimeout(debounceTimer);
  debounceTimer = null;
  try {
    await flushDirtyState();
    await invokeIpc('sticky_close_note', { id: note.value.id });
  } catch (err) {
    closing = false;
    const pending = contentDirty || themeDirty || windowDirty;
    if (pending) saveError.value = true;
    console.error(pending ? '关闭前保存便笺失败:' : '关闭便笺窗口失败:', err);
  }
}

/** 待办复选框点击处理 */
function handleContentClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (target && target.tagName === 'INPUT' && (target as HTMLInputElement).type === 'checkbox') {
    saveContentDebounced();
  }
}

if (typeof onUnmounted === 'function') {
  onUnmounted(() => {
    unlistenFlush?.();
  });
}

/** 初始化加载便笺，并在系统或主面板请求关闭时先保存再销毁。 */
onMounted(async () => {
  const targetId = props.noteId || new URLSearchParams(window.location.search).get('id') || '';
  // 必须在任何 await 之前记下占位值。会话或监听尚未完成时的输入不能被当成初始状态。
  const baseline = {
    title: note.value.title,
    content: note.value.content_md,
    theme: note.value.theme_color,
    opacity: note.value.opacity,
    pinned: note.value.is_pinned,
  };
  if (targetId) startWriteSession(targetId);
  const eventApi = (window as any).__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    try {
      unlistenFlush = await eventApi.listen(
        'sticky://flush-close',
        (event: { payload?: { id?: string } }) => {
          const requested = event?.payload?.id || '';
          if (!requested || requested === targetId || requested === note.value.id) {
            void closeNoteWindow();
          }
        },
      );
    } catch (err) {
      console.error('订阅便笺关闭保存失败:', err);
    }
  }
  if (!targetId) return;
  try {
    await sessionReady;
  } catch (err) {
    saveError.value = true;
    console.error('开始便笺写入会话失败:', err);
  }

  try {
    const data = await invokeIpc<StickyNoteRecord>('sticky_get_note', { id: targetId });
    if (!data) return;
    // 只保留加载期间用户真正改过的字段，避免用空白正文盖掉服务器上的内容。
    note.value = {
      ...data,
      title: note.value.title !== baseline.title ? note.value.title : data.title,
      content_md: note.value.content_md !== baseline.content ? note.value.content_md : data.content_md,
      theme_color: note.value.theme_color !== baseline.theme ? note.value.theme_color : data.theme_color,
      opacity: note.value.opacity !== baseline.opacity ? note.value.opacity : data.opacity,
      is_pinned: note.value.is_pinned !== baseline.pinned ? note.value.is_pinned : data.is_pinned,
    };
  } catch (err) {
    console.error('获取便笺详情失败:', err);
  }
});
</script>

<style scoped>
.sticky-header {
  height: 36px;
  background: rgba(128, 128, 128, 0.08);
  border-bottom: 1px solid var(--macos-border-separator);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px;
  -webkit-app-region: drag;
}

.sticky-header button,
.sticky-header input {
  -webkit-app-region: no-drag;
}

.header-left {
  flex: 1;
  min-width: 0;
  margin-right: 8px;
}

.title-input {
  width: 100%;
  background: transparent;
  border: none;
  color: inherit;
  font-size: 13px;
  font-weight: 600;
  outline: none;
}

.title-input:focus {
  border-bottom: 1px solid var(--macos-border-focus);
}

.header-right {
  display: flex;
  align-items: center;
  gap: 4px;
}

.tool-btn {
  background: transparent;
  border: none;
  color: var(--macos-text-secondary);
  border-radius: 4px;
  padding: 4px 6px;
  font-size: 11px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  transition: all 0.12s ease;
}

.tool-btn:hover {
  background: rgba(128, 128, 128, 0.15);
  color: var(--macos-text-primary);
}

.tool-btn.active {
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
}

.close-btn:hover {
  background: var(--macos-danger);
  color: #fff;
}

.palette-dropdown-wrap {
  position: relative;
}

.palette-popover {
  position: absolute;
  top: 26px;
  right: 0;
  background: var(--macos-bg-card);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid var(--macos-border-subtle);
  border-radius: 8px;
  padding: 6px;
  display: flex;
  gap: 6px;
  z-index: 100;
  box-shadow: var(--macos-shadow-popup);
}

.palette-dot {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 1px solid rgba(255, 255, 255, 0.3);
  cursor: pointer;
}

.palette-dot:hover {
  transform: scale(1.15);
}

.sticky-body {
  flex: 1;
  display: flex;
  overflow: hidden;
  position: relative;
}

.sticky-textarea {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  padding: 10px 12px;
  color: var(--macos-text-primary);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12.5px;
  line-height: 1.5;
  resize: none;
  user-select: text;
  -webkit-user-select: text;
}

.sticky-rendered-content {
  flex: 1;
  padding: 10px 14px;
  overflow-y: auto;
  font-size: 13px;
  line-height: 1.6;
  user-select: text;
  -webkit-user-select: text;
}

:deep(.katex-block-wrap) {
  margin: 8px 0;
  padding: 6px 10px;
  background: rgba(0, 0, 0, 0.2);
  border-radius: 6px;
  overflow-x: auto;
}

.sticky-footer {
  height: 26px;
  background: rgba(0, 0, 0, 0.18);
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  padding: 0 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10px;
  color: var(--macos-text-tertiary);
}

.opacity-slider-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
}

.opacity-range {
  width: 60px;
  height: 3px;
  accent-color: var(--macos-accent);
  cursor: pointer;
}

.save-status {
  font-size: 10px;
}
</style>
