<template>
  <div class="macos-window dashboard-frame">
    <header class="feature-header">
      <h1>{{ featureTitle }}</h1>
      <button class="link-button" @click="openHome">{{ t('desktop.allFeatures') }}</button>
      <button class="link-button" @click="openManual">{{ t('desktop.manual') }}</button>
    </header>
    <p v-if="error" class="feature-error" role="alert">{{ error }}</p>
    <main class="dashboard-content">
      <!-- Tab 2: 剪切板管理 (02 mod-clipboard) -->
      <section v-if="currentTab === 'clipboard'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="macos-search-box" style="max-width: 320px;">
            <Icons name="search" size="13" />
            <input
              v-model="clipSearchKeyword"
              type="text"
              class="macos-search-input"
              :placeholder="t('clipboard.searchPlaceholder')"
            />
          </div>

          <div class="toolbar-actions">
            <button class="macos-button" @click="clearUnpinnedClip">
              <Icons name="trash" size="12" />
              <span>{{ t('clipboard.clearUnpinned') }}</span>
            </button>
            <button class="macos-button primary" @click="openClipPopup">
              <Icons name="clipboard" size="12" />
              <span>{{ caption(t('clipboard.openPopupBtn'), 'clipboard:toggle_popup') }}</span>
            </button>
          </div>
        </div>

        <!-- 剪贴板历史表格 -->
        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th style="width: 60px;">{{ t('clipboard.colType') }}</th>
                  <th>{{ t('clipboard.colPreview') }}</th>
                  <th>{{ t('clipboard.colSource') }}</th>
                  <th>{{ t('clipboard.colUpdated') }}</th>
                  <th style="width: 140px;">{{ t('clipboard.colOperation') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="filteredClipItems.length === 0">
                  <td colspan="5" class="empty-cell">{{ t('clipboard.emptyTable') }}</td>
                </tr>
                <tr v-for="item in filteredClipItems" :key="item.id">
                  <td>
                    <span class="type-pill" :class="item.item_type">
                      {{ item.item_type === 'image' ? t('clipboard.typeImage') : t('clipboard.typeText') }}
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
                  <td>{{ item.source_app_name || t('clipboard.defaultSource') }}</td>
                  <td>{{ formatDateTime(item.updated_at) }}</td>
                  <td>
                    <div class="btn-group">
                      <button
                        class="macos-button"
                        :class="{ active: item.is_pinned }"
                        @click="toggleClipPin(item.id)"
                        :title="item.is_pinned ? t('clipboard.unpinTooltip') : t('clipboard.pinTooltip')"
                      >
                        <Icons :name="item.is_pinned ? 'pin-fill' : 'pin'" size="11" />
                      </button>
                      <button class="macos-button" @click="pasteClip(item.id)">
                        {{ t('clipboard.pasteBtn') }}
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

      <!-- Tab: 桌面屏幕注释 (03 mod-screen-ink) -->
      <section v-else-if="currentTab === 'screen_ink'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="card-desc">
            {{ t('screen_ink.dashboard.toolbarDesc') }}
          </div>
          <div class="toolbar-actions">
            <button
              class="macos-button"
              :disabled="!inkSessionStatus?.is_active"
              @click="toggleInkPassthrough"
            >
              <Icons name="mouse-pointer" size="12" />
              <span>{{ inkSessionStatus?.is_passthrough ? t('screen_ink.dashboard.resumeDrawBtn') : t('screen_ink.dashboard.passthroughBtn') }}</span>
            </button>
            <button
              class="macos-button"
              :disabled="!inkSessionStatus?.is_active"
              @click="clearInkCanvas"
            >
              <Icons name="trash" size="12" />
              <span>{{ t('screen_ink.dashboard.clearBtn') }}</span>
            </button>
            <button
              class="macos-button"
              :class="{ danger: inkSessionStatus?.is_active, primary: !inkSessionStatus?.is_active }"
              @click="toggleInkSession"
            >
              <Icons :name="inkSessionStatus?.is_active ? 'close' : 'brush'" size="12" />
              <span>{{ inkSessionStatus?.is_active ? t('screen_ink.dashboard.exitBtn') : t('screen_ink.dashboard.startBtn') }}</span>
            </button>
          </div>
        </div>

        <!-- 屏幕注释核心特性与运行状态卡片 -->
        <div class="section-card" style="margin-top: 10px;">
          <div class="card-header-bar">
            <span class="card-title">{{ t('screen_ink.dashboard.statusTitle') }}</span>
            <span class="card-desc">{{ t('screen_ink.dashboard.statusDesc') }}</span>
          </div>
          <div class="metrics-grid" style="grid-template-columns: repeat(2, 1fr); margin-top: 10px;">
            <div class="metric-card">
              <span class="metric-label">{{ t('screen_ink.dashboard.sessionStateLabel') }}</span>
              <span class="metric-val" :style="{ color: inkSessionStatus?.is_active ? '#10b981' : '#6b7280' }">
                {{ inkSessionStatus?.is_active ? t('screen_ink.dashboard.activeState') : t('screen_ink.dashboard.inactiveState') }}
              </span>
              <span class="metric-sub">{{ caption(t('screen_ink.dashboard.sessionStateSub'), 'screen_ink:toggle_session') }}</span>
            </div>
            <div class="metric-card">
              <span class="metric-label">{{ t('screen_ink.dashboard.canvasModeLabel') }}</span>
              <span class="metric-val" :style="{ color: canvasModeColor }">
                {{ canvasModeText }}
              </span>
              <span class="metric-sub">{{ caption(t('screen_ink.dashboard.canvasModeSub'), 'screen_ink:toggle_passthrough') }}</span>
            </div>

          </div>
        </div>

      </section>

      <!-- Tab 3: 置顶便笺库 (04 mod-sticky-note) -->
      <section v-else-if="currentTab === 'sticky'" class="tab-pane">
        <div class="pane-toolbar">
          <div class="card-desc">
            {{ t('sticky.toolbarDesc') }}
          </div>
          <button class="macos-button primary" @click="createNewNote">
            <Icons name="sticky" size="12" />
            <span>{{ caption(t('sticky.newNoteBtn'), 'sticky_note:create') }}</span>
          </button>
        </div>

        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th>{{ t('sticky.colTitle') }}</th>
                  <th>{{ t('sticky.colTheme') }}</th>
                  <th>{{ t('sticky.colStatus') }}</th>
                  <th>{{ t('sticky.colUpdated') }}</th>
                  <th style="width: 200px;">{{ t('sticky.colOperation') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="notes.length === 0">
                  <td colspan="5" class="empty-cell">{{ t('sticky.emptyTable') }}</td>
                </tr>
                <tr v-for="n in notes" :key="n.id">
                  <td><strong>{{ n.title || t('sticky.untitled') }}</strong></td>
                  <td>
                    <span class="theme-tag" :class="`theme-${n.theme_color}`">
                      {{ t(`sticky.window.themes.${n.theme_color}`) !== `sticky.window.themes.${n.theme_color}` ? t(`sticky.window.themes.${n.theme_color}`) : n.theme_color }}
                    </span>
                    <span style="font-size: 11px; color: var(--macos-text-secondary); margin-left: 6px;">
                      {{ Math.round(n.opacity * 100) }}%
                    </span>
                  </td>
                  <td>
                    <span class="status-pill" :class="{ open: n.is_open }">
                      {{ n.is_open ? t('sticky.statusOpen') : t('sticky.statusArchived') }}
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
                        {{ t('sticky.openWinBtn') }}
                      </button>
                      <button
                        v-else
                        class="macos-button"
                        @click="closeNote(n.id)"
                      >
                        {{ t('sticky.closeWinBtn') }}
                      </button>
                      <button class="macos-button" @click="exportNote(n.id)" :title="t('sticky.exportTooltip')">
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

      <!-- Tab 4: 自定义脚本 (06 mod-script-runner) -->
      <section v-else-if="currentTab === 'scripts'" class="tab-pane">
        <ScriptRunnerPanel />
      </section>

      <!-- Tab 5: 窗口置顶管理 (09 mod-window-pin) -->
      <section v-else-if="currentTab === 'winpin'" class="tab-pane">
        <p v-if="!windowPinSupported" class="section-card" style="padding: 12px;">
          {{ t('winpin.unsupportedNotice') }}
        </p>
        <div class="pane-toolbar">
          <div class="macos-search-box" style="max-width: 320px;">
            <Icons name="search" size="13" />
            <input
              v-model="winpinKeyword"
              type="text"
              class="macos-search-input"
              :placeholder="t('winpin.searchPlaceholder')"
            />
          </div>

          <div class="toolbar-actions">
            <button class="macos-button" @click="refreshDashboard" :title="t('winpin.refreshTooltip')">
              <Icons name="refresh" size="12" />
              <span>{{ t('winpin.refreshBtn') }}</span>
            </button>
            <button class="macos-button" @click="restoreAllPins">
              <Icons name="power" size="12" />
              <span>{{ t('winpin.restoreAllBtn') }}</span>
            </button>
            <button class="macos-button primary" @click="toggleForegroundPin" :disabled="!windowPinSupported">
              <Icons name="pin" size="12" />
              <span>{{ caption(t('desktop.pinForeground'), 'window_pin:toggle_foreground') }}</span>
            </button>
            <button class="macos-button" @click="openPinPicker" :disabled="!windowPinSupported">
              <Icons name="crosshair" size="12" />
              <span>{{ caption(t('desktop.pinPicker'), 'window_pin:pick_cursor') }}</span>
            </button>
          </div>
        </div>

        <div class="section-card" style="margin-top: 10px;">
          <div class="table-container">
            <table class="macos-table">
              <thead>
                <tr>
                  <th style="width: 80px;">{{ t('winpin.colId') }}</th>
                  <th>{{ t('winpin.colApp') }}</th>
                  <th>{{ t('winpin.colTitle') }}</th>
                  <th>{{ t('winpin.colGeometry') }}</th>
                  <th>{{ t('winpin.colStatus') }}</th>
                  <th style="width: 170px;">{{ t('winpin.colOperation') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="filteredExternalWindows.length === 0">
                  <td colspan="6" class="empty-cell">{{ t('winpin.emptyTable') }}</td>
                </tr>
                <tr v-for="win in filteredExternalWindows" :key="win.handle">
                  <td><code>#{{ win.handle }}</code></td>
                  <td><strong class="process-name" :title="win.process_name">{{ processName(win.process_name) }}</strong> <span class="pid-tag">PID {{ win.process_id }}</span></td>
                  <td>
                    <div class="table-text-cell">{{ win.title }}</div>
                  </td>
                  <td>{{ win.width }}×{{ win.height }} ({{ win.x }}, {{ win.y }})</td>
                  <td>
                    <span class="status-pill" :class="{ open: win.is_topmost }">
                      {{ win.is_topmost ? t('winpin.statusTopmost') : t('winpin.statusNormal') }}
                    </span>
                  </td>
                  <td>
                    <div class="btn-group">
                      <button
                        class="macos-button"
                        :class="{ primary: !win.is_topmost }"
                        @click="togglePinHandle(win.handle)"
                        :disabled="!windowPinSupported"
                      >
                        {{ win.is_topmost ? t('winpin.unpinBtn') : t('winpin.pinBtn') }}
                      </button>
                      <button class="macos-button" @click="activateWindow(win.handle)">
                        {{ t('winpin.activateBtn') }}
                      </button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <!-- Tab 10: 主密码加密密码本（macOS Touch ID 走钥匙串设备密钥） -->
      <section v-else-if="currentTab === 'vault'" class="tab-pane">
        <VaultPanel />
      </section>

      <!-- Tab 5: 基础多媒体处理 (08 mod-media-proc) -->
      <section v-else-if="currentTab === 'media'" class="tab-pane">
        <MediaPanel />
      </section>

      <!-- Tab 6: 应用使用时长 (05 mod-app-usage) -->
      <section v-else-if="currentTab === 'usage'" class="tab-pane">
        <AppUsagePanel />
      </section>
      <!-- 增强截图与贴图 (01 mod-screenshot) -->
      <section v-else-if="currentTab === 'screenshot'" class="tab-pane">
        <ScreenshotPanel />
      </section>
      <!-- Tab 6: 全局划词翻译 (07 mod-translator) -->
      <section v-else-if="currentTab === 'translator'" class="tab-pane">
        <TranslatorPanel />
      </section>
      <p v-if="!feature">{{ t('desktop.unavailable') }}</p>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, defineAsyncComponent } from 'vue';
import {
  invokeIpc,
  type KernelStatus,
  type ClipboardRecord,
  type StickyNoteRecord,
  type ExternalWindowInfo,
  type PinnedWindowEntry,
} from '../api/ipc';
import Icons from './Icons.vue';
const ScriptRunnerPanel = defineAsyncComponent(() => import('./ScriptRunnerPanel.vue'));
const MediaPanel = defineAsyncComponent(() => import('./MediaPanel.vue'));
const AppUsagePanel = defineAsyncComponent(() => import('./AppUsagePanel.vue'));
const ScreenshotPanel = defineAsyncComponent(() => import('./ScreenshotPanel.vue'));
const TranslatorPanel = defineAsyncComponent(() => import('./TranslatorPanel.vue'));
const VaultPanel = defineAsyncComponent(() => import('./VaultPanel.vue'));
import { t, syncLocaleFromBackendConfig } from '../utils/i18n';
import { featureForModule } from '../utils/features';
import { useActionShortcuts } from '../utils/actionShortcuts';
import { notify } from '../utils/toast';
const { caption } = useActionShortcuts();

const params = new URLSearchParams(window.location.search);
const feature = featureForModule((params.get('window') || '').replace(/^feature-/, ''));
const currentTab = ref(feature?.tab || '');
const error = ref('');
const featureTitle = computed(() => feature ? t(`moduleManager.names.${feature.module}`) : t('desktop.unavailable'));
async function openHome() { await invokeIpc('kernel_open_dashboard', { route: '/overview' }); }
async function openManual() {
  try { await invokeIpc('kernel_open_manual'); }
  catch (err) { error.value = String(err); }
}
function processName(path: string) { return path.split(/[\\/]/).pop() || path; }

/** 响应式状态数据 */
const status = ref<KernelStatus | null>(null);
const clipItems = ref<ClipboardRecord[]>([]);
const notes = ref<StickyNoteRecord[]>([]);
const externalWindows = ref<ExternalWindowInfo[]>([]);
const pinnedWindows = ref<PinnedWindowEntry[]>([]);
const inkSessionStatus = ref<{ is_active: boolean; is_passthrough: boolean } | null>(null);

/** 未启动会话时不能显示「墨迹绘制中」：只有会话进行中才区分绘制与穿透。 */
const canvasModeText = computed(() => {
  if (!inkSessionStatus.value?.is_active) return t('screen_ink.dashboard.idleMode');
  return inkSessionStatus.value.is_passthrough
    ? t('screen_ink.dashboard.passthroughMode')
    : t('screen_ink.dashboard.drawingMode');
});

const canvasModeColor = computed(() => {
  if (!inkSessionStatus.value?.is_active) return '#6b7280';
  return inkSessionStatus.value.is_passthrough ? '#3b82f6' : '#8b5cf6';
});

const moduleEnabled = (id: string) =>
  status.value?.module_catalog.some((module) => module.id === id && module.enabled) ?? false;
const windowPinSupported = computed(
  () => status.value?.module_catalog.find((module) => module.id === 'window_pin')?.supported ?? false
);

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
    const s = await invokeIpc<KernelStatus>('kernel_get_status');
    status.value = s;
    if (s.config?.general?.language) {
      syncLocaleFromBackendConfig(s.config.general.language);
    }
    const [c, n, w, i] = await Promise.all([
      currentTab.value === 'clipboard' && moduleEnabled('clipboard')
        ? invokeIpc<ClipboardRecord[]>('clipboard_list_items', { limit: 100 })
        : Promise.resolve([]),
      currentTab.value === 'sticky' && moduleEnabled('sticky_note')
        ? invokeIpc<StickyNoteRecord[]>('sticky_list_notes', { only_open: false })
        : Promise.resolve([]),
      currentTab.value === 'winpin' && moduleEnabled('window_pin')
        ? invokeIpc<{ candidates: ExternalWindowInfo[]; pinned: PinnedWindowEntry[] }>('winpin_list_candidates')
        : Promise.resolve({ candidates: [], pinned: [] }),
      currentTab.value === 'screen_ink' && moduleEnabled('screen_ink')
        ? invokeIpc<{ is_active: boolean; is_passthrough: boolean }>('ink_get_status')
        : Promise.resolve(null),
    ]);
    clipItems.value = c;
    notes.value = n;
    // Keep the managed pin visible even while another app temporarily changes Z-order.
    const managed = new Set((w.pinned || []).map(entry => entry.info.handle));
    externalWindows.value = (w.candidates || []).map(win => ({ ...win, is_topmost: win.is_topmost || managed.has(win.handle) }));
    pinnedWindows.value = w.pinned || [];
    inkSessionStatus.value = i;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
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

/** 切换屏幕注释会话开启/退出 */
async function toggleInkSession() {
  try {
    if (inkSessionStatus.value?.is_active) {
      await invokeIpc('ink_exit_session');
    } else {
      await invokeIpc('ink_start_session');
    }
    await refreshDashboard();
  } catch (err: any) {
    console.error('操作屏幕注释失败:', err);
  }
}

/** 切换画布鼠标穿透模式 */
async function toggleInkPassthrough() {
  try {
    const current = inkSessionStatus.value?.is_passthrough ?? false;
    await invokeIpc('ink_set_passthrough', { passthrough: !current });
    await refreshDashboard();
  } catch (err: any) {
    console.error('切换桌面穿透失败:', err);
  }
}

/** 清空屏幕墨迹 */
async function clearInkCanvas() {
  try {
    await invokeIpc('ink_clear_canvas');
    const bc = new BroadcastChannel('screen-ink-channel');
    bc.postMessage({ type: 'clear' });
    bc.close();
  } catch (err: any) {
    console.error('清空墨迹失败:', err);
  }
}

/** 新建便笺 */
async function createNewNote() {
  await invokeIpc('sticky_create_note', {
    title: t('sticky.defaultNewTitle'),
    content_md: t('sticky.defaultNewContent'),
    theme_color: 'neutral',
  });
  await refreshDashboard();
}

/** 唤醒打开便笺 */
async function openNote(id: string) {
  await invokeIpc('sticky_open_note', { id });
  await refreshDashboard();
}

/** 关闭便笺原生窗口。打开中的页面先保存未落盘内容，再自行销毁。 */
async function closeNote(id: string) {
  try {
    const res = await invokeIpc<{ delegated?: boolean }>('sticky_request_close', { id });
    if (!res?.delegated) await refreshDashboard();
  } catch (err) {
    console.error('关闭便笺窗口失败:', err);
  }
}

/** 彻底删除便笺 */
async function deleteNote(id: string) {
  await invokeIpc('sticky_delete_note', { id });
  await refreshDashboard();
}

/** 导出便笺 */
async function exportNote(id: string) {
  try {
    const chosen = await invokeIpc<{ path: string | null }>('sticky_pick_save_path', {
      default_name: `${t('sticky.exportFilePrefix')}_${id.slice(0, 8)}.md`,
      title: t('sticky.exportTooltip'), filter_label: 'Markdown (*.md)',
    });
    if (chosen.path) await invokeIpc('sticky_save_markdown', { id, path: chosen.path });
  } catch (err) {
    notify(String(err), true);
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

/** 格式化日期时间 */
function formatDateTime(ts: number) {
  const d = new Date(ts);
  const m = (d.getMonth() + 1).toString().padStart(2, '0');
  const day = d.getDate().toString().padStart(2, '0');
  const h = d.getHours().toString().padStart(2, '0');
  const min = d.getMinutes().toString().padStart(2, '0');
  return `${m}-${day} ${h}:${min}`;
}

let refreshTimer: ReturnType<typeof setInterval> | undefined;
let unlistenKernel: (() => void) | undefined;
onMounted(async () => {
  await refreshDashboard();
  if (['clipboard', 'sticky', 'winpin', 'screen_ink'].includes(currentTab.value)) {
    refreshTimer = setInterval(refreshDashboard, 2500);
  }
  const eventApi = (window as any).__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    unlistenKernel = await eventApi.listen('kernel://updated', () => void refreshDashboard());
  }
});
onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer);
  unlistenKernel?.();
});
</script>

<style scoped>
.dashboard-frame {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
}

.feature-header { display: flex; align-items: center; gap: 12px; padding: 14px 20px; border-bottom: 1px solid var(--macos-border-subtle); }
.feature-header h1 { font-size: 14px; font-weight: 600; flex: 1; }
.feature-header .link-button { background: none; border: 0; color: var(--macos-text-secondary); cursor: pointer; font-size: 12px; }
.feature-header .link-button:hover { color: var(--macos-accent); }
.feature-error { margin: 12px 20px 0; color: var(--macos-danger); font-size: 12px; }
.process-name { display: inline-block; max-width: 180px; overflow: hidden; text-overflow: ellipsis; vertical-align: middle; white-space: nowrap; }
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
  font-family: var(--app-font-sans);
}

.btn-group {
  display: flex;
  gap: 5px;
}

.theme-tag {
  font-size: 11px;
  padding: 2px 7px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
}

.status-pill {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.06);
  color: var(--macos-text-secondary);
}

.status-pill.open {
  background: rgba(48, 209, 88, 0.16);
  color: #30d158;
}

.pid-tag {
  font-size: 10px;
  color: var(--macos-text-tertiary);
}
</style>
