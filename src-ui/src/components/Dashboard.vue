<template>
  <div class="macos-window dashboard-frame">
    <!-- 顶部原生标题栏 -->
    <header class="macos-titlebar">
      <div class="macos-title">
        <Icons name="dashboard" size="14" />
        <span>All-in-One Toolbox</span>
        <span class="macos-badge-pill" :class="{ active: status?.active_window_count }">
          {{ status?.active_window_count || 0 }} 个原生窗口
        </span>
      </div>

      <!-- 分段选择器 (Segmented Control) 切换标签 -->
      <nav class="macos-segmented">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          class="macos-segment-item"
          :class="{ active: currentTab === tab.id }"
          @click="currentTab = tab.id"
        >
          <Icons :name="tab.icon" size="13" />
          <span>{{ tab.name }}</span>
        </button>
      </nav>

      <div class="title-right">
        <button class="macos-button danger" @click="shutdownDaemon" title="退出并清理所有置顶窗口">
          <Icons name="power" size="12" />
          <span>退出</span>
        </button>
      </div>
    </header>

    <!-- 主工作区 -->
    <main class="dashboard-content">
      <!-- Tab 1: 系统概览与运行指标 -->
      <section v-if="currentTab === 'overview'" class="tab-pane">
        <div class="metrics-grid">
          <div class="metric-card">
            <span class="metric-label">常驻内存级别</span>
            <span class="metric-val">Level 0 (静默守护)</span>
            <span class="metric-sub">关闭所有窗口时 0 WebView 驻留 (&lt;25MB)</span>
          </div>
          <div class="metric-card">
            <span class="metric-label">剪贴板历史容量</span>
            <span class="metric-val">{{ clipItems.length }} / {{ status?.config?.clipboard?.max_history_items || 100 }}</span>
            <span class="metric-sub">SHA-256 去重与 LRU 自动淘汰</span>
          </div>
          <div class="metric-card">
            <span class="metric-label">已置顶外部窗口</span>
            <span class="metric-val">{{ pinnedWindows.length }} 个</span>
            <span class="metric-sub">守护进程退出自动还原层级</span>
          </div>
          <div class="metric-card">
            <span class="metric-label">物理粘贴触发次数</span>
            <span class="metric-val">{{ status?.paste_trigger_count || 0 }} 次</span>
            <span class="metric-sub">enigo 模拟 Cmd+V / Ctrl+V</span>
          </div>
        </div>

        <!-- 全局快捷键注册表 -->
        <div class="section-card">
          <div class="card-header-bar">
            <span class="card-title">系统全局快捷键</span>
            <span class="card-desc">平台无关逻辑热键自动归一化</span>
          </div>
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th>所属模块</th>
                  <th>动作标识</th>
                  <th>当前物理快捷键</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="sc in status?.shortcuts || []" :key="sc.spec.action_id">
                  <td><code>{{ sc.spec.module_id }}</code></td>
                  <td><code>{{ sc.spec.action_id }}</code></td>
                  <td><kbd class="hotkey-pill">{{ sc.display_label }}</kbd></td>
                  <td>
                    <button class="macos-button" @click="triggerShortcut(sc.spec.action_id)">
                      测试触发
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <!-- 外部运行环境探测 -->
        <div class="section-card">
          <div class="card-header-bar">
            <span class="card-title">系统 PATH 运行时探测器 (EnvProbeService)</span>
            <span class="card-desc">零捆绑架构：按需探测并调用系统既有解释器</span>
          </div>
          <div class="probe-list">
            <div v-for="t in status?.env_tools || []" :key="t.tool" class="probe-item">
              <div class="probe-status-dot" :class="{ ready: t.available }"></div>
              <div class="probe-info">
                <span class="probe-name">{{ t.tool }}</span>
                <span class="probe-path">{{ t.resolved_path || '未在系统 PATH 中探测到' }}</span>
              </div>
              <span class="probe-badge">{{ t.available ? '已就绪' : '未检测到' }}</span>
            </div>
          </div>
        </div>
      </section>

      <!-- Tab 2: 剪切板管理 (02 mod-clipboard) -->
      <section v-else-if="currentTab === 'clipboard'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="macos-search-box" style="max-width: 320px;">
            <Icons name="search" size="13" />
            <input
              v-model="clipSearchKeyword"
              type="text"
              class="macos-search-input"
              placeholder="搜索剪贴板记录或来源应用..."
            />
          </div>

          <div class="toolbar-actions">
            <button class="macos-button" @click="clearUnpinnedClip">
              <Icons name="trash" size="12" />
              <span>清空未固定条目</span>
            </button>
            <button class="macos-button primary" @click="openClipPopup">
              <Icons name="clipboard" size="12" />
              <span>呼出剪贴板弹窗 (⇧⌘V)</span>
            </button>
          </div>
        </div>

        <!-- 剪贴板历史表格 -->
        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th style="width: 60px;">类型</th>
                  <th>内容预览</th>
                  <th>来源应用</th>
                  <th>更新时间</th>
                  <th style="width: 140px;">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="filteredClipItems.length === 0">
                  <td colspan="5" class="empty-cell">暂无剪贴板历史记录</td>
                </tr>
                <tr v-for="item in filteredClipItems" :key="item.id">
                  <td>
                    <span class="type-pill" :class="item.item_type">
                      {{ item.item_type === 'image' ? '图片' : '文本' }}
                    </span>
                  </td>
                  <td>
                    <template v-if="item.item_type === 'image'">
                      <img v-if="item.thumb_base64" :src="item.thumb_base64" class="dash-thumb" />
                    </template>
                    <template v-else>
                      <div class="table-text-cell">{{ item.content_text }}</div>
                    </template>
                  </td>
                  <td>{{ item.source_app_name || '系统剪贴板' }}</td>
                  <td>{{ formatDateTime(item.updated_at) }}</td>
                  <td>
                    <div class="btn-group">
                      <button
                        class="macos-button"
                        :class="{ active: item.is_pinned }"
                        @click="toggleClipPin(item.id)"
                        :title="item.is_pinned ? '取消固定' : '固定'"
                      >
                        <Icons :name="item.is_pinned ? 'pin-fill' : 'pin'" size="11" />
                      </button>
                      <button class="macos-button" @click="pasteClip(item.id)">
                        粘贴
                      </button>
                      <button class="macos-button danger" @click="deleteClip(item.id)">
                        <Icons name="trash" size="11" />
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <!-- Tab 3: 置顶便笺库 (04 mod-sticky-note) -->
      <section v-else-if="currentTab === 'sticky'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="card-desc">
            每个打开的便笺对应一个独立的置顶原生窗口；关闭便笺后标记为 <code>is_open = 0</code> 并立即销毁释放内存。
          </div>
          <button class="macos-button primary" @click="createNewNote">
            <Icons name="sticky" size="12" />
            <span>新建置顶便笺 (⇧⌘N)</span>
          </button>
        </div>

        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th>标题</th>
                  <th>主题色与透明度</th>
                  <th>桌面窗口状态</th>
                  <th>最后修改</th>
                  <th style="width: 200px;">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="notes.length === 0">
                  <td colspan="5" class="empty-cell">便笺库暂无记录，点击右上角新建</td>
                </tr>
                <tr v-for="n in notes" :key="n.id">
                  <td><strong>{{ n.title || '未命名便笺' }}</strong></td>
                  <td>
                    <span class="theme-tag" :class="`theme-${n.theme_color}`">
                      {{ n.theme_color }}
                    </span>
                    <span style="font-size: 11px; color: var(--macos-text-secondary); margin-left: 6px;">
                      {{ Math.round(n.opacity * 100) }}%
                    </span>
                  </td>
                  <td>
                    <span class="status-pill" :class="{ open: n.is_open }">
                      {{ n.is_open ? '桌面运行中' : '已归档关闭' }}
                    </span>
                  </td>
                  <td>{{ formatDateTime(n.updated_at) }}</td>
                  <td>
                    <div class="btn-group">
                      <button
                        v-if="!n.is_open"
                        class="macos-button"
                        @click="openNote(n.id)"
                      >
                        唤醒窗口
                      </button>
                      <button
                        v-else
                        class="macos-button"
                        @click="closeNote(n.id)"
                      >
                        关闭窗口
                      </button>
                      <button class="macos-button" @click="exportNote(n.id)" title="导出为 Markdown">
                        <Icons name="download" size="11" />
                      </button>
                      <button class="macos-button danger" @click="deleteNote(n.id)">
                        <Icons name="trash" size="11" />
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <!-- Tab 4: 窗口置顶管理 (09 mod-window-pin) -->
      <section v-else-if="currentTab === 'winpin'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="macos-search-box" style="max-width: 320px;">
            <Icons name="search" size="13" />
            <input
              v-model="winpinKeyword"
              type="text"
              class="macos-search-input"
              placeholder="过滤当前系统真实窗口..."
            />
          </div>

          <div class="toolbar-actions">
            <button class="macos-button" @click="restoreAllPins">
              <Icons name="power" size="12" />
              <span>一键还原全部置顶</span>
            </button>
            <button class="macos-button primary" @click="toggleForegroundPin">
              <Icons name="pin" size="12" />
              <span>置顶当前活动窗口 (⇧⌘P)</span>
            </button>
            <button class="macos-button" @click="openPinPicker">
              <Icons name="crosshair" size="12" />
              <span>准星拾取 (⌥⌘P)</span>
            </button>
          </div>
        </div>

        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th style="width: 80px;">窗口 ID</th>
                  <th>应用程序</th>
                  <th>窗口标题</th>
                  <th>位置尺寸</th>
                  <th>置顶状态</th>
                  <th style="width: 170px;">操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="filteredExternalWindows.length === 0">
                  <td colspan="6" class="empty-cell">未检测到符合条件的外部窗口</td>
                </tr>
                <tr v-for="win in filteredExternalWindows" :key="win.handle">
                  <td><code>#{{ win.handle }}</code></td>
                  <td><strong>{{ win.process_name }}</strong> <span class="pid-tag">PID {{ win.process_id }}</span></td>
                  <td>
                    <div class="table-text-cell">{{ win.title }}</div>
                  </td>
                  <td>{{ win.width }}×{{ win.height }} ({{ win.x }}, {{ win.y }})</td>
                  <td>
                    <span class="status-pill" :class="{ open: win.is_topmost }">
                      {{ win.is_topmost ? '置顶最前中' : '普通层级' }}
                    </span>
                  </td>
                  <td>
                    <div class="btn-group">
                      <button
                        class="macos-button"
                        :class="{ primary: !win.is_topmost }"
                        @click="togglePinHandle(win.handle)"
                      >
                        {{ win.is_topmost ? '取消置顶' : '置顶' }}
                      </button>
                      <button class="macos-button" @click="activateWindow(win.handle)">
                        激活
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue';
import {
  invokeIpc,
  type KernelStatus,
  type ClipboardRecord,
  type StickyNoteRecord,
  type ExternalWindowInfo,
  type PinnedWindowEntry,
} from '../api/ipc';
import Icons from './Icons.vue';

/** 选项卡配置列表 */
const tabs = [
  { id: 'overview', name: '总览', icon: 'dashboard' },
  { id: 'clipboard', name: '02. 剪贴板', icon: 'clipboard' },
  { id: 'sticky', name: '04. 便笺库', icon: 'sticky' },
  { id: 'winpin', name: '09. 窗口置顶', icon: 'pin' },
];

/** 当前选中的选项卡 ID */
const currentTab = ref('overview');

/** 响应式状态数据 */
const status = ref<KernelStatus | null>(null);
const clipItems = ref<ClipboardRecord[]>([]);
const notes = ref<StickyNoteRecord[]>([]);
const externalWindows = ref<ExternalWindowInfo[]>([]);
const pinnedWindows = ref<PinnedWindowEntry[]>([]);

/** 过滤搜索词 */
const clipSearchKeyword = ref('');
const winpinKeyword = ref('');

/** 计算过滤后的剪贴板列表 */
const filteredClipItems = computed(() => {
  const kw = clipSearchKeyword.value.trim().toLowerCase();
  if (!kw) return clipItems.value;
  return clipItems.value.filter((i) => {
    return (
      (i.content_text && i.content_text.toLowerCase().includes(kw)) ||
      (i.source_app_name && i.source_app_name.toLowerCase().includes(kw))
    );
  });
});

/** 计算过滤后的外部窗口列表 */
const filteredExternalWindows = computed(() => {
  const kw = winpinKeyword.value.trim().toLowerCase();
  if (!kw) return externalWindows.value;
  return externalWindows.value.filter((w) => {
    return (
      w.process_name.toLowerCase().includes(kw) ||
      w.title.toLowerCase().includes(kw)
    );
  });
});

/** 刷新全部内核与模块数据 */
async function refreshDashboard() {
  try {
    const [s, c, n, w] = await Promise.all([
      invokeIpc<KernelStatus>('kernel_get_status'),
      invokeIpc<ClipboardRecord[]>('clipboard_list_items', { limit: 100 }),
      invokeIpc<StickyNoteRecord[]>('sticky_list_notes', { only_open: false }),
      invokeIpc<{ candidates: ExternalWindowInfo[]; pinned: PinnedWindowEntry[] }>('winpin_list_candidates'),
    ]);
    status.value = s;
    clipItems.value = c;
    notes.value = n;
    externalWindows.value = w.candidates || [];
    pinnedWindows.value = w.pinned || [];
  } catch (err) {
    console.error('刷新主控台数据失败:', err);
  }
}

/** 触发快捷键动作 */
async function triggerShortcut(actionId: string) {
  try {
    await invokeIpc('kernel_trigger_shortcut', { action_id: actionId });
    await refreshDashboard();
  } catch (err) {
    console.error('触发动作失败:', err);
  }
}

/** 打开剪贴板弹窗 */
async function openClipPopup() {
  await triggerShortcut('clipboard:toggle_popup');
}

/** 切换剪贴板固定状态 */
async function toggleClipPin(id: number) {
  await invokeIpc('clipboard_toggle_pin', { id });
  await refreshDashboard();
}

/** 粘贴剪贴板条目 */
async function pasteClip(id: number) {
  await invokeIpc('clipboard_paste_item', { id, as_plain_text: false });
  await refreshDashboard();
}

/** 删除单条剪贴板条目 */
async function deleteClip(id: number) {
  await invokeIpc('clipboard_delete_item', { id });
  await refreshDashboard();
}

/** 清空所有未固定的剪贴板记录 */
async function clearUnpinnedClip() {
  await invokeIpc('clipboard_clear_unpinned');
  await refreshDashboard();
}

/** 新建便笺 */
async function createNewNote() {
  await invokeIpc('sticky_create_note', {
    title: '新建便笺',
    content_md: '支持 **Markdown** 与 LaTeX 公式：$E=mc^2$\n\n$$\\int_a^b f(x)dx$$',
    theme_color: 'neutral',
  });
  await refreshDashboard();
}

/** 唤醒打开便笺 */
async function openNote(id: string) {
  await invokeIpc('sticky_open_note', { id });
  await refreshDashboard();
}

/** 关闭便笺原生窗口 */
async function closeNote(id: string) {
  await invokeIpc('sticky_close_note', { id });
  await refreshDashboard();
}

/** 彻底删除便笺 */
async function deleteNote(id: string) {
  await invokeIpc('sticky_delete_note', { id });
  await refreshDashboard();
}

/** 导出便笺 */
async function exportNote(id: string) {
  try {
    const res = await invokeIpc<{ id: string; markdown: string }>('sticky_export_markdown', { id });
    const blob = new Blob([res.markdown], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `便笺_${id.slice(0, 8)}.md`;
    link.click();
    URL.revokeObjectURL(url);
  } catch (err) {
    console.error('导出失败:', err);
  }
}

/** 置顶当前活动窗口 */
async function toggleForegroundPin() {
  await invokeIpc('winpin_toggle_foreground');
  await refreshDashboard();
}

/** 打开准星拾取层 */
async function openPinPicker() {
  await triggerShortcut('window_pin:pick_cursor');
}

/** 切换指定窗口置顶 */
async function togglePinHandle(handle: number) {
  await invokeIpc('winpin_toggle_handle', { handle });
  await refreshDashboard();
}

/** 激活原窗口 */
async function activateWindow(handle: number) {
  await invokeIpc('winpin_activate', { handle });
}

/** 还原所有已置顶外部窗口 */
async function restoreAllPins() {
  await invokeIpc('winpin_restore_all');
  await refreshDashboard();
}

/** 退出微内核守护进程 */
async function shutdownDaemon() {
  await invokeIpc('kernel_shutdown');
  (window as any).__TAURI__?.window?.getCurrentWindow?.()?.destroy?.();
}

/** 格式化日期时间 */
function formatDateTime(ts: number) {
  const d = new Date(ts);
  const m = (d.getMonth() + 1).toString().padStart(2, '0');
  const day = d.getDate().toString().padStart(2, '0');
  const h = d.getHours().toString().padStart(2, '0');
  const min = d.getMinutes().toString().padStart(2, '0');
  return `${m}-${day} ${h}:${min}`;
}

onMounted(async () => {
  await refreshDashboard();
  setInterval(() => {
    refreshDashboard();
  }, 1000);
});
</script>

<style scoped>
.dashboard-frame {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
}

.title-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.dashboard-content {
  flex: 1;
  overflow-y: auto;
  padding: 16px 20px;
}

.tab-pane {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: 10px;
}

.metric-card {
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-md);
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
}

.metric-label {
  font-size: 11px;
  color: var(--macos-text-secondary);
}

.metric-val {
  font-size: 18px;
  font-weight: 600;
  color: var(--macos-text-primary);
  margin: 4px 0 2px 0;
}

.metric-sub {
  font-size: 10px;
  color: var(--macos-text-tertiary);
}

.section-card {
  background: var(--macos-bg-card);
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-md);
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.card-header-bar {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
}

.card-title {
  font-size: 13px;
  font-weight: 600;
}

.card-desc {
  font-size: 11px;
  color: var(--macos-text-tertiary);
}

.table-container {
  overflow-x: auto;
}

.macos-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.macos-table th {
  text-align: left;
  padding: 7px 10px;
  color: var(--macos-text-secondary);
  border-bottom: 1px solid var(--macos-border-separator);
  font-weight: 500;
}

.macos-table td {
  padding: 8px 10px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  color: var(--macos-text-primary);
}

.empty-cell {
  text-align: center;
  color: var(--macos-text-tertiary);
  padding: 24px 0;
}

.hotkey-pill {
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 4px;
  padding: 2px 6px;
  font-family: inherit;
  font-size: 11px;
}

.probe-list {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 8px;
}

.probe-item {
  background: rgba(0, 0, 0, 0.2);
  border: 1px solid var(--macos-border-subtle);
  border-radius: 6px;
  padding: 8px 10px;
  display: flex;
  align-items: center;
  gap: 10px;
}

.probe-status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6e6e73;
}

.probe-status-dot.ready {
  background: var(--macos-success);
  box-shadow: 0 0 6px rgba(48, 209, 88, 0.4);
}

.probe-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.probe-name {
  font-weight: 600;
  font-size: 12px;
}

.probe-path {
  font-size: 10px;
  color: var(--macos-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.probe-badge {
  font-size: 10px;
  color: var(--macos-text-secondary);
}

.pane-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.toolbar-actions {
  display: flex;
  gap: 8px;
}

.type-pill {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.08);
  color: var(--macos-text-secondary);
}

.type-pill.image {
  background: rgba(10, 132, 255, 0.15);
  color: #38bdf8;
}

.dash-thumb {
  max-width: 60px;
  max-height: 36px;
  border-radius: 4px;
  object-fit: cover;
  border: 1px solid var(--macos-border-subtle);
}

.table-text-cell {
  max-width: 320px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.btn-group {
  display: flex;
  gap: 4px;
}

.status-pill {
  font-size: 11px;
  padding: 2px 7px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.06);
  color: var(--macos-text-tertiary);
}

.status-pill.open {
  background: var(--macos-success-subtle);
  color: var(--macos-success);
}

.theme-tag {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.08);
}

.pid-tag {
  font-size: 10px;
  color: var(--macos-text-tertiary);
  margin-left: 4px;
}
</style>
