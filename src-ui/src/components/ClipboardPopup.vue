<template>
  <div class="macos-window clipboard-popup-container" @keydown="handleKeyNavigation" tabindex="0">
    <!-- 顶部标题栏（展示产品矢量 Logo 与多语言标题） -->
    <header class="macos-titlebar">
      <div class="macos-title">
        <ProductLogo :size="16" />
        <span>{{ t('clipboard.popup.title') }}</span>
        <span class="macos-badge-pill active">{{ t('clipboard.popup.countBadge', { count: items.length }) }}</span>
      </div>
      <button class="icon-button" @click="closePopup" :title="t('clipboard.popup.closeTooltip')">
        <Icons name="close" size="12" />
      </button>
    </header>

    <!-- 搜索输入框 -->
    <div class="search-section">
      <div class="macos-search-box">
        <Icons name="search" size="13" />
        <input
          ref="searchInputRef"
          v-model="searchQuery"
          type="text"
          class="macos-search-input"
          :placeholder="t('clipboard.popup.searchPlaceholder')"
          @keydown.enter.prevent="pasteCurrentOrFirst"
        />
      </div>
    </div>

    <!-- 剪贴板条目列表 -->
    <div class="clip-scroll-list">
      <div v-if="filteredItems.length === 0" class="empty-hint">
        <span>{{ t('clipboard.popup.emptyHint') }}</span>
      </div>

      <div
        v-for="(item, index) in filteredItems"
        :key="item.id"
        class="clip-card"
        :class="{ 'is-active': index === selectedIndex, 'is-pinned': item.is_pinned }"
        @click="selectedIndex = index"
        @dblclick="pasteItem(item.id)"
      >
        <!-- 左侧类型标记与主体内容 -->
        <div class="card-main">
          <!-- 文本类型内容 -->
          <template v-if="item.item_type === 'text'">
            <div class="text-preview">{{ item.content_text }}</div>
          </template>

          <!-- 真实图片缩略图 -->
          <template v-else-if="item.item_type === 'image'">
            <div class="image-preview-wrap">
              <img
                v-if="item.thumb_base64"
                :src="item.thumb_base64"
                :alt="t('clipboard.popup.imageAlt')"
                class="thumb-img"
              />
              <span class="image-label">{{ t('clipboard.popup.imageLabel') }}</span>
            </div>
          </template>

          <!-- 元数据：应用来源与时间 -->
          <div class="card-meta">
            <span class="app-tag">{{ item.source_app_name || t('clipboard.defaultSource') }}</span>
            <span class="time-tag">{{ formatTime(item.updated_at) }}</span>
            <span v-if="item.is_pinned" class="pinned-tag">{{ t('clipboard.popup.pinnedBadge') }}</span>
          </div>
        </div>

        <!-- 右侧快捷操作按钮 -->
        <div class="card-actions">
          <button
            class="action-btn"
            :class="{ active: item.is_pinned }"
            @click.stop="togglePin(item.id)"
            :title="item.is_pinned ? t('clipboard.popup.unpinItemTooltip') : t('clipboard.popup.pinItemTooltip')"
          >
            <Icons :name="item.is_pinned ? 'pin-fill' : 'pin'" size="13" />
          </button>
          <button
            class="action-btn paste-btn"
            @click.stop="pasteItem(item.id)"
            :title="t('clipboard.popup.pasteTooltip')"
          >
            {{ t('clipboard.pasteBtn') }}
          </button>
        </div>
      </div>
    </div>

    <!-- 底部极简状态提示栏 -->
    <footer class="popup-footer">
      <span>{{ t('clipboard.popup.footerKeys') }}</span>
      <span>{{ t('clipboard.popup.footerArch') }}</span>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, nextTick } from 'vue';
import { invokeIpc, type ClipboardRecord } from '../api/ipc';
import Icons from './Icons.vue';
import ProductLogo from './ProductLogo.vue';
import { t } from '../utils/i18n';

/** 响应式剪贴板数据列表 */
const items = ref<ClipboardRecord[]>([]);
/** 搜索过滤关键词 */
const searchQuery = ref('');
/** 键盘当前选中的索引项 */
const selectedIndex = ref(0);
/** 搜索输入框 DOM 引用 */
const searchInputRef = ref<HTMLInputElement | null>(null);

/** 计算过滤后的条目列表 */
const filteredItems = computed(() => {
  const kw = searchQuery.value.trim().toLowerCase();
  if (!kw) return items.value;
  return items.value.filter((item) => {
    if (item.content_text && item.content_text.toLowerCase().includes(kw)) {
      return true;
    }
    if (item.source_app_name && item.source_app_name.toLowerCase().includes(kw)) {
      return true;
    }
    return false;
  });
});

/** 刷新剪贴板列表 */
async function loadItems() {
  try {
    const list = await invokeIpc<ClipboardRecord[]>('clipboard_list_items', { limit: 100 });
    items.value = list;
    if (selectedIndex.value >= list.length) {
      selectedIndex.value = 0;
    }
  } catch (err) {
    console.error('加载剪贴板历史失败:', err);
  }
}

/** 切换置顶固定状态 */
async function togglePin(id: number) {
  try {
    await invokeIpc('clipboard_toggle_pin', { id });
    await loadItems();
  } catch (err) {
    console.error('切换固定失败:', err);
  }
}

/** 粘贴选中的条目到目标应用前台窗口 */
async function pasteItem(id: number) {
  try {
    await invokeIpc('clipboard_paste_item', { id, as_plain_text: false });
  } catch (err) {
    console.error('粘贴上屏失败:', err);
  }
}

/** 回车执行粘贴：优先粘贴键盘高亮选中项，若无则粘贴首项 */
async function pasteCurrentOrFirst() {
  const currentList = filteredItems.value;
  if (currentList.length === 0) return;
  const target = currentList[selectedIndex.value] || currentList[0];
  if (target) {
    await pasteItem(target.id);
  }
}

/** 销毁关闭弹窗 */
async function closePopup() {
  try {
    await invokeIpc('kernel_destroy_window', { label: 'clipboard-popup' });
  } catch (err) {
    console.error('关闭剪贴板弹窗失败:', err);
  }
}

/** 键盘方向键快捷选择 */
function handleKeyNavigation(e: KeyboardEvent) {
  // Esc 是窗口生命周期操作，不能被“空列表/无搜索结果”的导航短路拦截。
  if (e.key === 'Escape') {
    e.preventDefault();
    closePopup();
    return;
  }
  const listLen = filteredItems.value.length;
  if (listLen === 0) return;

  if (e.key === 'ArrowDown') {
    e.preventDefault();
    selectedIndex.value = (selectedIndex.value + 1) % listLen;
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    selectedIndex.value = (selectedIndex.value - 1 + listLen) % listLen;
  }
}

/** 时间格式化为自然显示 */
function formatTime(timestampMs: number): string {
  const d = new Date(timestampMs);
  const now = new Date();
  const diffSec = Math.floor((now.getTime() - d.getTime()) / 1000);
  if (diffSec < 60) return t('clipboard.popup.justNow');
  if (diffSec < 3600) return t('clipboard.popup.minutesAgo', { minutes: Math.floor(diffSec / 60) });
  const hours = d.getHours().toString().padStart(2, '0');
  const minutes = d.getMinutes().toString().padStart(2, '0');
  return `${hours}:${minutes}`;
}

onMounted(async () => {
  await loadItems();
  nextTick(() => {
    searchInputRef.value?.focus();
  });
});
</script>

<style scoped>
.clipboard-popup-container {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  /* 跟随全局主题，避免浅色模式使用深色背景而文字仍为深色。 */
  background: var(--macos-bg-window);
  backdrop-filter: blur(28px) saturate(180%);
  border: 1px solid rgba(255, 255, 255, 0.1);
  outline: none;
}

.icon-button {
  border: none;
  background: transparent;
  color: var(--macos-text-secondary);
  cursor: pointer;
  padding: 4px;
  border-radius: 4px;
  display: flex;
  align-items: center;
}

.icon-button:hover {
  background: var(--macos-bg-card-hover);
  color: var(--macos-text-primary);
}

.search-section {
  padding: 8px 12px;
  border-bottom: 1px solid var(--macos-border-separator);
}

.clip-scroll-list {
  flex: 1;
  overflow-y: auto;
  padding: 6px 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.empty-hint {
  display: flex;
  justify-content: center;
  align-items: center;
  height: 120px;
  color: var(--macos-text-tertiary);
  font-size: 12px;
}

.clip-card {
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  border-radius: 6px;
  padding: 8px 10px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  cursor: pointer;
  transition: all 0.12s ease;
}

.clip-card:hover {
  background: var(--macos-bg-card-hover);
  border-color: var(--macos-border-subtle);
}

.clip-card.is-active {
  border-color: var(--macos-accent);
  background: var(--macos-accent-subtle);
}

.clip-card.is-pinned {
  border-left: 3px solid #ff9f0a;
}

.card-main {
  flex: 1;
  min-width: 0;
}

.text-preview {
  font-size: 12px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--macos-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  word-break: break-all;
}

.image-preview-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
}

.thumb-img {
  max-width: 90px;
  max-height: 48px;
  object-fit: cover;
  border-radius: 4px;
  border: 1px solid var(--macos-border-subtle);
  background: rgba(128, 128, 128, 0.1);
}

.image-label {
  font-size: 11px;
  color: var(--macos-text-secondary);
}

.card-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  font-size: 10px;
  color: var(--macos-text-tertiary);
}

.app-tag {
  background: rgba(128, 128, 128, 0.1);
  padding: 1px 5px;
  border-radius: 3px;
}

.pinned-tag {
  color: #ff9f0a;
  font-weight: 500;
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.action-btn {
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  color: var(--macos-text-secondary);
  border-radius: 4px;
  padding: 4px 6px;
  font-size: 11px;
  cursor: pointer;
  display: flex;
  align-items: center;
}

.action-btn:hover {
  background: var(--macos-bg-card-hover);
  color: var(--macos-text-primary);
}

.action-btn.active {
  color: #ff9f0a;
}

.paste-btn {
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
  padding: 4px 8px;
  font-weight: 500;
}

.paste-btn:hover {
  background: var(--macos-accent);
  color: #ffffff;
}

.popup-footer {
  height: 24px;
  padding: 0 12px;
  border-top: 1px solid var(--macos-border-separator);
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 10px;
  color: var(--macos-text-tertiary);
}
</style>
