<template>
  <div class="app-usage">
    <p v-if="error" class="usage-error" role="alert">{{ error }}</p>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('appUsage.title') }}</span>
          <span class="card-desc">{{ t('appUsage.desc') }}</span>
        </div>
        <div class="toolbar-actions">
          <span class="status-pill" :class="{ open: status?.tracking && canCount }">
            {{ status?.tracking && canCount ? t('appUsage.trackingOn') : t('appUsage.trackingOff') }}
          </span>
          <button class="macos-button" @click="loadAll" :disabled="loading">
            <Icons name="refresh" size="12" />
            <span>{{ t('appUsage.refresh') }}</span>
          </button>
        </div>
      </div>

      <p
        v-if="status"
        class="usage-limit"
        :class="{ available: audioAvailable }"
        role="status"
      >
        {{ audioStatusText }}
      </p>
      <p v-if="status && !canCount" class="usage-limit" role="status">
        {{ t('appUsage.unavailableState') }}
      </p>

      <div class="usage-range-bar">
        <nav class="macos-segmented">
          <button
            v-for="option in rangeOptions"
            :key="option.days"
            class="macos-segment-item"
            :class="{ active: rangeDays === option.days }"
            @click="selectRange(option.days)"
          >
            {{ t(option.labelKey) }}
          </button>
        </nav>
        <label class="usage-date-field">
          <span>{{ t('appUsage.startDate') }}</span>
          <input v-model="startDate" type="date" class="macos-input" @change="loadOverview" />
        </label>
        <label class="usage-date-field">
          <span>{{ t('appUsage.endDate') }}</span>
          <input v-model="endDate" type="date" class="macos-input" @change="loadOverview" />
        </label>
      </div>
    </div>

    <div class="metrics-grid usage-metrics">
      <div class="metric-card">
        <span class="metric-label">{{ t('appUsage.totalLabel') }}</span>
        <span class="metric-val">{{ formatDuration(overview?.total_seconds || 0) }}</span>
        <span class="metric-sub">{{ t('appUsage.totalSub') }}</span>
      </div>
      <div class="metric-card">
        <span class="metric-label">{{ t('appUsage.averageLabel') }}</span>
        <span class="metric-val">{{ formatDuration(averageSeconds) }}</span>
        <span class="metric-sub">{{ t('appUsage.averageSub') }}</span>
      </div>
      <div class="metric-card">
        <span class="metric-label">{{ t('appUsage.topAppLabel') }}</span>
        <span class="metric-val usage-top-app">{{ topApp?.app_name || t('appUsage.noData') }}</span>
        <span class="metric-sub">
          {{ topApp ? formatDuration(topApp.active_seconds) : t('appUsage.topAppSub') }}
        </span>
      </div>
      <div class="metric-card">
        <span class="metric-label">{{ t('appUsage.activeDaysLabel') }}</span>
        <span class="metric-val">{{ activeDays }}</span>
        <span class="metric-sub">{{ t('appUsage.activeDaysSub', { count: overview?.days.length || 0 }) }}</span>
      </div>
    </div>

    <div class="usage-columns">
      <div class="section-card">
        <div class="card-header-bar">
          <div>
            <span class="card-title">{{ t('appUsage.dailyTitle') }}</span>
            <span class="card-desc">{{ t('appUsage.dailyDesc') }}</span>
          </div>
        </div>
        <div v-if="!overview || overview.days.length === 0" class="empty-cell">
          {{ t('appUsage.noData') }}
        </div>
        <div v-else class="usage-day-list">
          <div v-for="day in overview.days" :key="day.date" class="usage-day-row">
            <span class="usage-day-label">{{ day.date }}</span>
            <div class="usage-bar-track">
              <div
                class="usage-bar-fill"
                :style="{ width: `${percent(day.active_seconds, dailyMax)}%` }"
              ></div>
            </div>
            <span class="usage-day-value">{{ formatDuration(day.active_seconds) }}</span>
          </div>
        </div>
      </div>

      <div class="section-card">
        <div class="card-header-bar">
          <div>
            <span class="card-title">{{ t('appUsage.categoryTitle') }}</span>
            <span class="card-desc">{{ t('appUsage.categoryDesc') }}</span>
          </div>
        </div>
        <div v-if="!overview || overview.categories.length === 0" class="empty-cell">
          {{ t('appUsage.noData') }}
        </div>
        <div v-else class="usage-category-list">
          <div v-for="item in overview.categories" :key="item.category" class="usage-category-row">
            <span class="usage-category-name">{{ categoryLabel(item.category) }}</span>
            <div class="usage-bar-track">
              <div
                class="usage-bar-fill category"
                :style="{ width: `${percent(item.active_seconds, overview.total_seconds)}%` }"
              ></div>
            </div>
            <span class="usage-day-value">{{ formatDuration(item.active_seconds) }}</span>
          </div>
        </div>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('appUsage.hourlyTitle') }}</span>
          <span class="card-desc">{{ t('appUsage.hourlyDesc') }}</span>
        </div>
        <label class="usage-date-field">
          <span>{{ t('appUsage.hourlyDate') }}</span>
          <input v-model="hourlyDate" type="date" class="macos-input" @change="loadHourly" />
        </label>
      </div>
      <div class="usage-hour-grid">
        <div v-for="hour in hourly?.hours || []" :key="hour.hour" class="usage-hour-cell">
          <div class="usage-hour-bar-wrap">
            <div
              class="usage-hour-bar"
              :style="{ height: `${percent(hour.active_seconds, hourlyMax)}%` }"
              :title="`${hour.hour}:00 · ${formatDuration(hour.active_seconds)}`"
            ></div>
          </div>
          <span class="usage-hour-label">{{ hour.hour }}</span>
        </div>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('appUsage.sessionsTitle') }}</span>
          <span class="card-desc">{{ t('appUsage.sessionsDesc') }}</span>
        </div>
        <div class="usage-session-controls">
          <label class="usage-date-field">
            <span>{{ t('appUsage.sessionDate') }}</span>
            <input
              v-model="sessionDate"
              type="date"
              class="macos-input"
              @change="resetSessions"
            />
          </label>
          <button
            class="macos-button"
            :disabled="sessionOffset === 0"
            @click="changeSessionPage(-1)"
          >
            {{ t('appUsage.prevPage') }}
          </button>
          <span class="usage-page-info">
            {{ t('appUsage.pageInfo', { page: sessionPageNumber, pages: sessionPageCount }) }}
          </span>
          <button
            class="macos-button"
            :disabled="!sessionHasNext"
            @click="changeSessionPage(1)"
          >
            {{ t('appUsage.nextPage') }}
          </button>
        </div>
      </div>
      <div class="table-container">
        <table class="macos-table">
          <thead>
            <tr>
              <th>{{ t('appUsage.colApp') }}</th>
              <th>{{ t('appUsage.colWindowTitle') }}</th>
              <th>{{ t('appUsage.colStart') }}</th>
              <th>{{ t('appUsage.colEnd') }}</th>
              <th>{{ t('appUsage.colDuration') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="sessionPage.sessions.length === 0">
              <td colspan="5" class="empty-cell">{{ t('appUsage.noSessions') }}</td>
            </tr>
            <tr v-for="session in sessionPage.sessions" :key="session.id">
              <td>
                <strong>{{ session.app_name }}</strong>
                <div class="usage-app-path">{{ session.app_id }}</div>
              </td>
              <td class="usage-session-title">
                {{ session.window_title || t('appUsage.titleHidden') }}
              </td>
              <td>{{ formatTimestamp(session.started_at) }}</td>
              <td>{{ formatTimestamp(session.ended_at) }}</td>
              <td>{{ formatDuration(session.active_seconds) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('appUsage.appsTitle') }}</span>
          <span class="card-desc">{{ t('appUsage.appsDesc') }}</span>
        </div>
      </div>
      <div class="table-container">
        <table class="macos-table">
          <thead>
            <tr>
              <th>{{ t('appUsage.colApp') }}</th>
              <th>{{ t('appUsage.colId') }}</th>
              <th>{{ t('appUsage.colCategory') }}</th>
              <th>{{ t('appUsage.colIgnored') }}</th>
              <th>{{ t('appUsage.colLastSeen') }}</th>
              <th>{{ t('appUsage.colTotal') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="apps.length === 0">
              <td colspan="6" class="empty-cell">{{ t('appUsage.noData') }}</td>
            </tr>
            <tr v-for="app in apps" :key="app.app_id">
              <td>
                <strong>{{ app.app_name }}</strong>
                <div class="usage-app-path">{{ app.executable_path || app.app_id }}</div>
              </td>
              <td><code class="usage-app-id">{{ app.app_id }}</code></td>
              <td>
                <select
                  class="macos-input"
                  :value="app.category"
                  @change="changeCategory(app, ($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="category in categories" :key="category" :value="category">
                    {{ categoryLabel(category) }}
                  </option>
                </select>
              </td>
              <td>
                <input
                  type="checkbox"
                  :checked="app.ignored"
                  @change="toggleIgnored(app, ($event.target as HTMLInputElement).checked)"
                />
              </td>
              <td>{{ formatTimestamp(app.last_seen_at) }}</td>
              <td>{{ formatDuration(app.total_seconds) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div class="section-card">
      <div class="card-header-bar">
        <div>
          <span class="card-title">{{ t('appUsage.settingsTitle') }}</span>
          <span class="card-desc">{{ t('appUsage.settingsDesc') }}</span>
        </div>
      </div>
      <div class="usage-settings">
        <label class="usage-setting-field">
          <span>{{ t('appUsage.idleThreshold') }}</span>
          <input v-model.number="settingsForm.idle_threshold_secs" type="number" min="30" class="macos-input" />
        </label>
        <label class="usage-setting-field">
          <span>{{ t('appUsage.flushInterval') }}</span>
          <input v-model.number="settingsForm.flush_interval_secs" type="number" min="5" class="macos-input" />
        </label>
        <label class="usage-setting-check">
          <input v-model="settingsForm.keep_window_titles" type="checkbox" />
          <span>{{ t('appUsage.keepTitles') }}</span>
        </label>
        <label class="usage-setting-check">
          <input
            v-model="settingsForm.audio_afk_exception"
            type="checkbox"
            :disabled="!status?.capabilities.audio_afk_exception"
          />
          <span>{{ t('appUsage.audioToggle') }}</span>
        </label>
        <button class="macos-button primary" @click="saveSettings">
          {{ t('appUsage.saveSettings') }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import {
  invokeIpc,
  type AppUsageConfig,
  type AppUsageStatus,
  type HourlyBreakdown,
  type TrackedApp,
  type UsageOverview,
  type UsageSessionPage,
} from '../api/ipc';
import { t } from '../utils/i18n';
import Icons from './Icons.vue';

const error = ref('');
const loading = ref(false);
const status = ref<AppUsageStatus | null>(null);
const canCount = computed(() => status.value?.capabilities.active_app
  && status.value?.capabilities.idle_detection
  && status.value?.capabilities.screen_lock_detection);
const overview = ref<UsageOverview | null>(null);
const hourly = ref<HourlyBreakdown | null>(null);
const apps = ref<TrackedApp[]>([]);
const sessionPage = ref<UsageSessionPage>({
  date: '',
  total: 0,
  offset: 0,
  limit: 20,
  sessions: [],
});
const SESSION_PAGE_SIZE = 20;

const rangeOptions = [
  { days: 7, labelKey: 'appUsage.range7' },
  { days: 30, labelKey: 'appUsage.range30' },
];
const categories = [
  'work',
  'development',
  'communication',
  'browse',
  'media',
  'game',
  'system',
  'other',
];

const rangeDays = ref(7);
const today = toDateInput(new Date());
const startDate = ref(addDays(today, -6));
const endDate = ref(today);
const hourlyDate = ref(today);
const sessionDate = ref(today);
const sessionOffset = ref(0);
const settingsForm = reactive<AppUsageConfig>({
  idle_threshold_secs: 300,
  flush_interval_secs: 60,
  keep_window_titles: true,
  audio_afk_exception: true,
});

const audioAvailable = computed(
  () =>
    (status.value?.capabilities.audio_afk_exception ?? false)
    && (status.value?.config.audio_afk_exception ?? false)
);
const audioStatusText = computed(() => {
  if (!status.value) return '';
  if (!status.value.capabilities.audio_afk_exception) {
    return t('appUsage.audioLimit');
  }
  if (!status.value.config.audio_afk_exception) {
    return t('appUsage.audioDisabled');
  }
  return t('appUsage.audioReady');
});
const sessionHasNext = computed(
  () => sessionPage.value.offset + sessionPage.value.limit < sessionPage.value.total
);
const sessionPageNumber = computed(() =>
  sessionPage.value.total === 0
    ? 1
    : Math.floor(sessionPage.value.offset / sessionPage.value.limit) + 1
);
const sessionPageCount = computed(() =>
  Math.max(1, Math.ceil(sessionPage.value.total / sessionPage.value.limit))
);

const averageSeconds = computed(() => {
  const total = overview.value?.total_seconds || 0;
  const days = Math.max(overview.value?.days.length || 1, 1);
  return Math.round(total / days);
});
const activeDays = computed(
  () => overview.value?.days.filter((day) => day.active_seconds > 0).length || 0
);
const topApp = computed(() => overview.value?.apps[0]);
const dailyMax = computed(() =>
  Math.max(1, ...(overview.value?.days || []).map((day) => day.active_seconds))
);
const hourlyMax = computed(() =>
  Math.max(1, ...(hourly.value?.hours || []).map((hour) => hour.active_seconds))
);

function toDateInput(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, '0');
  const day = `${date.getDate()}`.padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function addDays(date: string, days: number): string {
  const parsed = new Date(`${date}T12:00:00`);
  parsed.setDate(parsed.getDate() + days);
  return toDateInput(parsed);
}

function percent(value: number, max: number): number {
  if (!Number.isFinite(value) || max <= 0) return 0;
  return Math.max(2, Math.min(100, (value / max) * 100));
}

function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hours > 0) {
    return `${hours}${t('appUsage.hourShort')} ${minutes}${t('appUsage.minuteShort')}`;
  }
  if (minutes > 0) {
    return `${minutes}${t('appUsage.minuteShort')} ${secs}${t('appUsage.secondShort')}`;
  }
  return `${secs}${t('appUsage.secondShort')}`;
}

function formatTimestamp(millis: number): string {
  if (!millis) return '—';
  return new Date(millis).toLocaleString();
}

function categoryLabel(category: string): string {
  return t(`appUsage.categories.${category}`);
}

function selectRange(days: number) {
  rangeDays.value = days;
  endDate.value = today;
  startDate.value = addDays(today, -(days - 1));
  void loadOverview();
}

async function loadStatus() {
  status.value = await invokeIpc<AppUsageStatus>('usage_get_status');
  Object.assign(settingsForm, status.value.config);
}

async function loadOverview() {
  overview.value = await invokeIpc<UsageOverview>('usage_get_overview', {
    start_date: startDate.value,
    end_date: endDate.value,
  });
}

async function loadHourly() {
  hourly.value = await invokeIpc<HourlyBreakdown>('usage_get_hourly', {
    date: hourlyDate.value,
  });
}

async function loadApps() {
  apps.value = await invokeIpc<TrackedApp[]>('usage_get_apps');
}

async function loadSessions() {
  sessionPage.value = await invokeIpc<UsageSessionPage>('usage_get_sessions', {
    date: sessionDate.value,
    offset: sessionOffset.value,
    limit: SESSION_PAGE_SIZE,
  });
}

function resetSessions() {
  sessionOffset.value = 0;
  void loadSessions();
}

function changeSessionPage(direction: number) {
  const nextOffset = sessionOffset.value + direction * SESSION_PAGE_SIZE;
  if (nextOffset < 0) return;
  sessionOffset.value = nextOffset;
  void loadSessions();
}

async function loadAll() {
  error.value = '';
  loading.value = true;
  try {
    await Promise.all([loadStatus(), loadOverview(), loadHourly(), loadApps(), loadSessions()]);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function changeCategory(app: TrackedApp, category: string) {
  try {
    await invokeIpc<TrackedApp>('usage_set_app_category', {
      app_id: app.app_id,
      category,
      ignored: app.ignored,
    });
    await Promise.all([loadOverview(), loadHourly(), loadApps()]);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function toggleIgnored(app: TrackedApp, ignored: boolean) {
  try {
    await invokeIpc<TrackedApp>('usage_set_app_category', {
      app_id: app.app_id,
      category: app.category,
      ignored,
    });
    await Promise.all([loadOverview(), loadHourly(), loadApps()]);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

async function saveSettings() {
  error.value = '';
  try {
    const config = await invokeIpc<AppUsageConfig>('usage_update_config', {
      idle_threshold_secs: settingsForm.idle_threshold_secs,
      flush_interval_secs: settingsForm.flush_interval_secs,
      keep_window_titles: settingsForm.keep_window_titles,
      audio_afk_exception: settingsForm.audio_afk_exception,
    });
    Object.assign(settingsForm, config);
    await loadStatus();
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

onMounted(() => {
  void loadAll();
});
</script>

<style scoped>
.usage-error {
  margin: 0 0 10px;
  padding: 8px 12px;
  border-radius: 8px;
  color: var(--macos-danger, #b91c1c);
  background: rgba(185, 28, 28, 0.08);
}

.usage-limit {
  margin: 10px 0 0;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 12px;
  background: rgba(245, 158, 11, 0.12);
  color: var(--macos-text-secondary, #52525b);
}

.usage-limit.available {
  background: rgba(52, 199, 89, 0.12);
}

.usage-session-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 8px;
}

.usage-page-info {
  font-size: 11px;
  color: var(--macos-text-secondary, #71717a);
  align-self: center;
}

.usage-session-title {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.usage-range-bar,
.usage-settings {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 12px;
  margin-top: 12px;
}

.usage-date-field,
.usage-setting-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11px;
  color: var(--macos-text-secondary, #71717a);
}

.usage-setting-check {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.usage-metrics {
  margin-top: 10px;
}

.usage-top-app {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.usage-columns {
  display: grid;
  grid-template-columns: minmax(0, 1.1fr) minmax(0, 0.9fr);
  gap: 10px;
}

@media (max-width: 900px) {
  .usage-columns {
    grid-template-columns: 1fr;
  }
}

.usage-day-list,
.usage-category-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 360px;
  overflow: auto;
  padding: 4px 0;
}

.usage-day-row,
.usage-category-row {
  display: grid;
  grid-template-columns: 92px minmax(0, 1fr) 88px;
  align-items: center;
  gap: 10px;
  font-size: 12px;
}

.usage-day-label {
  font-variant-numeric: tabular-nums;
  color: var(--macos-text-secondary, #52525b);
}

.usage-day-value {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.usage-bar-track {
  height: 8px;
  border-radius: 999px;
  background: var(--macos-fill-quaternary, rgba(120, 120, 128, 0.16));
  overflow: hidden;
}

.usage-bar-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--macos-accent, #0a84ff);
  transition: width 0.2s ease;
}

.usage-bar-fill.category {
  background: linear-gradient(90deg, #34c759, #0a84ff);
}

.usage-hour-grid {
  display: grid;
  grid-template-columns: repeat(24, minmax(0, 1fr));
  gap: 4px;
  align-items: end;
  padding: 12px 4px 0;
  min-height: 150px;
}

.usage-hour-cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  min-width: 0;
}

.usage-hour-bar-wrap {
  width: 100%;
  height: 110px;
  display: flex;
  align-items: flex-end;
}

.usage-hour-bar {
  width: 100%;
  min-height: 2px;
  border-radius: 4px 4px 2px 2px;
  background: var(--macos-accent, #0a84ff);
}

.usage-hour-label {
  font-size: 9px;
  color: var(--macos-text-secondary, #71717a);
}

.usage-app-path {
  font-size: 10px;
  color: var(--macos-text-tertiary, #8e8e93);
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.usage-app-id {
  font-size: 10px;
  word-break: break-all;
}
</style>
