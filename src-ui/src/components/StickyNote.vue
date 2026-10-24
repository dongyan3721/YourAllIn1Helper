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
          placeholder="未命名便笺..."
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
          :title="viewMode === 'edit' ? '切换为预览渲染' : '切换为编辑模式'"
        >
          <Icons :name="viewMode === 'edit' ? 'text' : 'check'" size="12" />
          <span>{{ viewMode === 'edit' ? '预览' : '编辑' }}</span>
        </button>

        <!-- 置顶开关 -->
        <button
          class="tool-btn"
          :class="{ active: note.is_pinned }"
          @click="togglePin"
          title="置顶最前 (Always on Top)"
        >
          <Icons :name="note.is_pinned ? 'pin-fill' : 'pin'" size="12" />
        </button>

        <!-- 调色盘弹出切换 -->
        <div class="palette-dropdown-wrap">
          <button class="tool-btn" @click="showPalette = !showPalette" title="便笺主题色">
            <Icons name="palette" size="12" />
          </button>
          <div v-if="showPalette" class="palette-popover">
            <button
              v-for="color in themeOptions"
              :key="color.key"
              class="palette-dot"
              :style="{ background: color.color }"
              :title="color.label"
              @click="setTheme(color.key)"
            ></button>
          </div>
        </div>

        <!-- 导出 Markdown -->
        <button class="tool-btn" @click="exportMarkdownFile" title="导出为 .md 文件">
          <Icons name="download" size="12" />
        </button>

        <!-- 关闭并销毁原生窗口 -->
        <button class="tool-btn close-btn" @click="closeNoteWindow" title="关闭便笺 (归还内存)">
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
        placeholder="输入 Markdown 内容或 LaTeX 公式 ($E=mc^2$)..."
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
        <span class="opacity-label">透明度 {{ Math.round(note.opacity * 100) }}%</span>
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
        <span v-if="saving">正在存盘...</span>
        <span v-else-if="lastSavedTime">已保存 {{ lastSavedTime }}</span>
      </div>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue';
import { marked } from 'marked';
import katex from 'katex';
import 'katex/dist/katex.min.css';
import { invokeIpc, type StickyNoteRecord } from '../api/ipc';
import Icons from './Icons.vue';

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
/** 上次存盘时间 */
const lastSavedTime = ref('');
/** 编辑器输入引用 */
const editorRef = ref<HTMLTextAreaElement | null>(null);

/** 主题色彩列表 */
const themeOptions = [
  { key: 'neutral', label: '冷淡中性灰', color: '#2c2c2e' },
  { key: 'amber', label: '琥珀暗暖', color: '#451a03' },
  { key: 'sky', label: '深海幽蓝', color: '#082f49' },
  { key: 'emerald', label: '青苔雅绿', color: '#064e3b' },
  { key: 'rose', label: '暗夜玫瑰', color: '#4c0519' },
  { key: 'violet', label: '暮光深紫', color: '#2e1065' },
];

/**
 * 核心渲染器：将文本中的 Markdown 语法与 LaTeX 数学公式渲染为标准 HTML
 */
const renderedHtml = computed(() => {
  let content = note.value.content_md || '';
  if (!content.trim()) {
    return '<p style="color:#6e6e73;font-style:italic">暂无内容，点击上方「编辑」开始记录...</p>';
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

/** 防抖计时器 */
let debounceTimer: any = null;

/** 执行防抖保存 */
function saveContentDebounced() {
  clearTimeout(debounceTimer);
  saving.value = true;
  debounceTimer = setTimeout(async () => {
    try {
      await invokeIpc('sticky_save_content', {
        id: note.value.id,
        title: note.value.title,
        content_md: note.value.content_md,
      });
      const now = new Date();
      const h = now.getHours().toString().padStart(2, '0');
      const m = now.getMinutes().toString().padStart(2, '0');
      const s = now.getSeconds().toString().padStart(2, '0');
      lastSavedTime.value = `${h}:${m}:${s}`;
    } catch (err) {
      console.error('保存便笺失败:', err);
    } finally {
      saving.value = false;
    }
  }, 400);
}

/** 切换编辑/预览视图 */
function toggleViewMode() {
  viewMode.value = viewMode.value === 'edit' ? 'preview' : 'edit';
}

/** 切换置顶状态 */
async function togglePin() {
  note.value.is_pinned = !note.value.is_pinned;
  try {
    await invokeIpc('sticky_update_window_state', {
      id: note.value.id,
      is_pinned: note.value.is_pinned,
    });
  } catch (err) {
    console.error('更新置顶状态失败:', err);
  }
}

/** 调节透明度 */
async function onOpacityChange(e: Event) {
  const val = Number((e.target as HTMLInputElement).value);
  const opacity = Math.max(0.35, Math.min(1.0, val / 100.0));
  note.value.opacity = opacity;
  try {
    await invokeIpc('sticky_update_window_state', {
      id: note.value.id,
      opacity,
    });
  } catch (err) {
    console.error('更新透明度失败:', err);
  }
}

/** 切换色彩主题 */
async function setTheme(colorKey: string) {
  note.value.theme_color = colorKey;
  showPalette.value = false;
  try {
    await invokeIpc('sticky_update_theme', {
      id: note.value.id,
      theme_color: colorKey,
    });
  } catch (err) {
    console.error('更新主题色失败:', err);
  }
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
    link.download = `${note.value.title || '便笺'}_${note.value.id.slice(0, 8)}.md`;
    link.click();
    URL.revokeObjectURL(url);
  } catch (err) {
    console.error('导出便笺失败:', err);
  }
}

/** 关闭并销毁便笺原生 WebView 窗口 */
async function closeNoteWindow() {
  try {
    await invokeIpc('sticky_close_note', { id: note.value.id });
  } catch (err) {
    console.error('关闭便笺窗口失败:', err);
  }
}

/** 待办复选框点击处理 */
function handleContentClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (target && target.tagName === 'INPUT' && (target as HTMLInputElement).type === 'checkbox') {
    // 待办点击切换
    saveContentDebounced();
  }
}

/** 初始化加载便笺 */
onMounted(async () => {
  const targetId = props.noteId || new URLSearchParams(window.location.search).get('id') || '';
  if (!targetId) return;

  try {
    const data = await invokeIpc<StickyNoteRecord>('sticky_get_note', { id: targetId });
    if (data) {
      note.value = data;
    }
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
