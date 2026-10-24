<template>
  <div class="media-panel">
    <header class="media-heading">
      <div>
        <h2>{{ t('media.title') }}</h2>
        <p>{{ t('media.subtitle') }}</p>
      </div>
      <button class="macos-button" :disabled="initializing" @click="initialize">
        <Icons name="refresh" :size="12" />
        <span>{{ t('media.actions.refresh') }}</span>
      </button>
    </header>

    <div v-if="notice" class="media-notice" :class="noticeKind" role="status">
      <span>{{ notice }}</span>
      <button class="notice-close" :aria-label="t('media.actions.dismiss')" @click="notice = ''">
        <Icons name="close" :size="12" />
      </button>
    </div>

    <section class="media-card environment-card">
      <div class="card-heading">
        <div>
          <h3>{{ t('media.environment.title') }}</h3>
          <p>{{ t('media.environment.description') }}</p>
        </div>
        <span class="availability-badge" :class="{ ready: avReady }">
          {{ avReady ? t('media.environment.ready') : t('media.environment.unavailable') }}
        </span>
      </div>
      <div class="probe-grid">
        <div v-for="probe in environmentProbes" :key="probe.name" class="probe-item">
          <span class="probe-dot" :class="{ ready: probe.available }"></span>
          <div class="probe-content">
            <div class="probe-title-row">
              <strong>{{ probe.name }}</strong>
              <span>{{ probe.available ? t('media.environment.detected') : t('media.environment.missing') }}</span>
            </div>
            <span v-if="probe.version_summary" class="probe-detail">{{ probe.version_summary }}</span>
            <span v-if="probe.resolved_path" class="probe-detail probe-path" :title="probe.resolved_path">
              {{ probe.resolved_path }}
            </span>
            <span v-if="!probe.available" class="probe-hint">
              {{ probe.install_hint || t('media.environment.installHint') }}
            </span>
          </div>
        </div>
      </div>
      <p v-if="environment && !avReady" class="environment-warning">
        {{ t('media.environment.avDisabled') }}
      </p>
    </section>

    <section class="media-card output-card">
      <div class="card-heading compact">
        <div>
          <h3>{{ t('media.output.title') }}</h3>
          <p>{{ t('media.output.description') }}</p>
        </div>
        <button class="macos-button" :disabled="savingConfig" @click="saveConfig">
          <Icons name="check" :size="12" />
          <span>{{ savingConfig ? t('media.actions.saving') : t('media.actions.save') }}</span>
        </button>
      </div>
      <div class="output-settings">
        <label class="field output-path-field">
          <span>{{ t('media.output.directory') }}</span>
          <div class="input-action-row">
            <input
              v-model="outputDir"
              class="media-input"
              type="text"
              :placeholder="t('media.output.defaultDirectory')"
              spellcheck="false"
            />
            <button class="macos-button" @click="pickOutputDirectory">
              {{ t('media.actions.chooseDirectory') }}
            </button>
          </div>
        </label>
        <label class="field concurrency-field">
          <span>{{ t('media.output.maxConcurrent') }}</span>
          <input
            v-model.number="maxConcurrentJobs"
            class="media-input"
            type="number"
            min="1"
            max="4"
            step="1"
          />
        </label>
      </div>
    </section>

    <div class="media-workspace">
      <section class="media-card source-card">
        <div class="card-heading compact">
          <div>
            <h3>{{ t('media.files.title') }}</h3>
            <p>{{ t('media.files.selected', { count: pendingFiles.length }) }}</p>
          </div>
          <div class="button-row">
            <button class="macos-button" @click="pickFiles">
              <Icons name="photo" :size="12" />
              <span>{{ t('media.actions.addFiles') }}</span>
            </button>
            <button class="macos-button" :disabled="pendingFiles.length === 0" @click="clearFiles">
              <Icons name="trash" :size="12" />
              <span>{{ t('media.actions.clear') }}</span>
            </button>
          </div>
        </div>

        <div class="file-list" :class="{ empty: pendingFiles.length === 0 }">
          <div v-if="pendingFiles.length === 0" class="empty-files">
            <Icons name="download" :size="24" />
            <strong>{{ t('media.files.emptyTitle') }}</strong>
            <span>{{ t('media.files.dropHint') }}</span>
          </div>
          <div v-for="(path, index) in pendingFiles" :key="path" class="file-row">
            <div class="file-icon"><Icons name="sticky" :size="14" /></div>
            <div class="file-info">
              <strong :title="path">{{ fileName(path) }}</strong>
              <span :title="path">{{ path }}</span>
            </div>
            <div class="file-actions">
              <button
                class="small-button"
                :disabled="(!isImageFile(path) && !environment?.ffprobe.available) || inspectingPath !== null"
                :title="t('media.actions.inspect')"
                @click="inspectFile(path)"
              >
                {{ inspectingPath === path ? t('media.actions.inspecting') : t('media.actions.inspect') }}
              </button>
              <button
                class="icon-button"
                :aria-label="t('media.actions.remove')"
                :title="t('media.actions.remove')"
                @click="removeFile(index)"
              >
                <Icons name="close" :size="12" />
              </button>
            </div>
          </div>
        </div>
      </section>

      <section class="media-card options-card">
        <div class="card-heading compact">
          <div>
            <h3>{{ t('media.options.title') }}</h3>
            <p>{{ t('media.options.description') }}</p>
          </div>
        </div>

        <nav class="macos-segmented mode-switch" :aria-label="t('media.options.mode')">
          <button class="macos-segment-item" :class="{ active: mode === 'image' }" @click="mode = 'image'">
            <Icons name="photo" :size="12" />
            <span>{{ t('media.options.imageMode') }}</span>
          </button>
          <button class="macos-segment-item" :class="{ active: mode === 'av' }" @click="mode = 'av'">
            <Icons name="dashboard" :size="12" />
            <span>{{ t('media.options.avMode') }}</span>
          </button>
        </nav>

        <div v-if="mode === 'image'" class="options-form">
          <label class="field">
            <span>{{ t('media.image.format') }}</span>
            <select v-model="imageFormat" class="media-input">
              <option v-for="format in imageFormats" :key="format" :value="format">
                {{ format.toUpperCase() }}
              </option>
            </select>
          </label>
          <label v-if="imageFormat === 'jpeg'" class="field quality-field">
            <span>{{ t('media.image.quality', { value: imageQuality }) }}</span>
            <div class="quality-controls">
              <input v-model.number="imageQuality" type="range" min="1" max="100" step="1" />
              <input v-model.number="imageQuality" class="media-input" type="number" min="1" max="100" />
            </div>
          </label>
          <p v-else-if="imageFormat === 'webp'" class="option-note">{{ t('media.image.losslessNote') }}</p>
          <div class="dimension-grid">
            <label class="field">
              <span>{{ t('media.image.width') }}</span>
              <input
                v-model.number="imageWidth"
                class="media-input"
                type="number"
                min="1"
                step="1"
                :placeholder="t('media.options.auto')"
              />
            </label>
            <label class="field">
              <span>{{ t('media.image.height') }}</span>
              <input
                v-model.number="imageHeight"
                class="media-input"
                type="number"
                min="1"
                step="1"
                :placeholder="t('media.options.auto')"
              />
            </label>
          </div>
          <label class="checkbox-field">
            <input v-model="preserveAspect" type="checkbox" />
            <span>{{ t('media.image.preserveAspect') }}</span>
          </label>
        </div>

        <div v-else class="options-form">
          <label class="field">
            <span>{{ t('media.av.operation') }}</span>
            <select v-model="avOperation" class="media-input">
              <option value="convert">{{ t('media.operations.convert') }}</option>
              <option value="trim">{{ t('media.operations.trim') }}</option>
              <option value="extract_audio">{{ t('media.operations.extractAudio') }}</option>
              <option value="adjust_volume">{{ t('media.operations.adjustVolume') }}</option>
            </select>
          </label>
          <label v-if="!isTrim" class="field">
            <span>{{ t('media.av.outputFormat') }}</span>
            <select v-model="avOutputFormat" class="media-input">
              <option v-for="format in currentAvFormats" :key="format" :value="format">
                {{ format.toUpperCase() }}
              </option>
            </select>
          </label>
          <div v-if="isTrim" class="dimension-grid">
            <label class="field">
              <span>{{ t('media.av.startSeconds') }}</span>
              <input v-model.number="startSeconds" class="media-input" type="number" min="0" step="0.1" />
            </label>
            <label class="field">
              <span>{{ t('media.av.endSeconds') }}</span>
              <input
                v-model.number="endSeconds"
                class="media-input"
                type="number"
                min="0"
                step="0.1"
                :placeholder="t('media.av.endPlaceholder')"
              />
            </label>
          </div>
          <label v-show="isAdjustVolume" class="field">
            <span>{{ t('media.av.volume') }}</span>
            <div class="quality-controls">
              <input v-model.number="volume" type="range" min="0" max="4" step="0.05" />
              <input v-model.number="volume" class="media-input" type="number" min="0" max="4" step="0.05" />
            </div>
          </label>
          <label v-if="isTrim" class="checkbox-field">
            <input v-model="precise" type="checkbox" />
            <span>{{ t('media.av.precise') }}</span>
          </label>
          <p v-if="!avReady" class="option-note warning">{{ t('media.environment.avDisabled') }}</p>
        </div>

        <button
          class="macos-button primary submit-button"
          :disabled="pendingFiles.length === 0 || submitting || (mode === 'av' && !avReady)"
          @click="submitBatch"
        >
          <span>{{ submitting ? t('media.actions.submitting') : t('media.actions.submitBatch', { count: pendingFiles.length }) }}</span>
        </button>
      </section>
    </div>

    <section class="media-card jobs-card">
      <div class="card-heading compact">
        <div>
          <h3>{{ t('media.jobs.title') }}</h3>
          <p>{{ t('media.jobs.description') }}</p>
        </div>
        <span class="job-count">{{ t('media.jobs.count', { count: jobs.length }) }}</span>
      </div>
      <div v-if="jobs.length === 0" class="empty-jobs">{{ t('media.jobs.empty') }}</div>
      <div v-else class="job-list">
        <article v-for="job in jobs" :key="job.id" class="job-item">
          <div class="job-topline">
            <div class="job-title">
              <span class="operation-badge">{{ operationLabel(job.operation) }}</span>
              <strong :title="job.input_path">{{ fileName(job.input_path) }}</strong>
            </div>
            <div class="job-actions">
              <span class="status-badge" :class="job.status">{{ statusLabel(job.status) }}</span>
              <button
                v-if="job.status === 'queued' || job.status === 'running'"
                class="small-button danger"
                :disabled="cancellingJobs.has(job.id)"
                @click="cancelJob(job.id)"
              >
                {{ cancellingJobs.has(job.id) ? t('media.actions.cancelling') : t('media.actions.cancel') }}
              </button>
            </div>
          </div>
          <div class="job-path" :title="job.input_path">{{ job.input_path }}</div>
          <div class="progress-row">
            <div
              class="progress-track"
              role="progressbar"
              :aria-label="t('media.jobs.progress')"
              :aria-valuenow="normalizedProgress(job.progress)"
              aria-valuemin="0"
              aria-valuemax="100"
            >
              <div class="progress-value" :class="job.status" :style="{ width: `${normalizedProgress(job.progress)}%` }"></div>
            </div>
            <span class="progress-number">{{ normalizedProgress(job.progress) }}%</span>
            <span v-if="job.speed" class="job-speed">{{ job.speed }}</span>
          </div>
          <p v-if="job.error" class="job-error">{{ t('media.jobs.error', { message: job.error }) }}</p>
          <div v-if="job.output_path" class="job-output">
            <span>{{ t('media.jobs.output') }}</span>
            <code :title="job.output_path">{{ job.output_path }}</code>
          </div>
          <template v-if="job.metadata">
            <div v-if="metadataSummary(job).length" class="metadata-summary">
              <span v-for="item in metadataSummary(job)" :key="item">{{ item }}</span>
            </div>
            <details class="metadata-details">
              <summary>{{ t('media.inspect.showJson') }}</summary>
              <pre>{{ formatMetadata(job.metadata) }}</pre>
            </details>
          </template>
          <div class="job-time">{{ t('media.jobs.updated', { time: formatDateTime(job.updated_at) }) }}</div>
        </article>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invokeIpc } from '../api/ipc';
import { t } from '../utils/i18n';
import Icons from './Icons.vue';

type ImageFormat = 'png' | 'jpeg' | 'webp' | 'bmp' | 'ico';
type AvOperation = 'convert' | 'trim' | 'extract_audio' | 'adjust_volume';
type JobStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

interface ToolProbe {
  available: boolean;
  resolved_path: string | null;
  version_summary: string | null;
  install_hint: string | null;
}

interface MediaEnvironment {
  ffmpeg: ToolProbe;
  ffprobe: ToolProbe;
  image_formats: string[];
  av_operations: string[];
}

interface MediaConfig {
  output_dir: string | null;
  max_concurrent_jobs: number;
  history_limit: number;
}

interface MediaJob {
  id: string;
  operation: string;
  input_path: string;
  output_path: string | null;
  status: JobStatus;
  progress: number;
  speed: string | null;
  error: string | null;
  metadata: any;
  created_at: number;
  updated_at: number;
}

interface DragDropPayload {
  paths: string[];
}

interface JobProgressPayload {
  job_id: string;
  job?: MediaJob;
  progress: number;
  speed: string | null;
}

const imageFormats: ImageFormat[] = ['png', 'jpeg', 'webp', 'bmp', 'ico'];
const convertFormats = ['mp4', 'mkv', 'webm', 'mov', 'mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg'];
const audioFormats = ['mp3', 'aac', 'flac', 'wav', 'm4a', 'ogg'];

const environment = ref<MediaEnvironment | null>(null);
const pendingFiles = ref<string[]>([]);
const jobs = ref<MediaJob[]>([]);
const outputDir = ref('');
const maxConcurrentJobs = ref(2);
const mode = ref<'image' | 'av'>('image');
const imageFormat = ref<ImageFormat>('png');
const imageQuality = ref(85);
const imageWidth = ref<number | null>(null);
const imageHeight = ref<number | null>(null);
const preserveAspect = ref(true);
const avOperation = ref<AvOperation>('convert');
const avOutputFormat = ref('mp4');
const startSeconds = ref<number | null>(0);
const endSeconds = ref<number | null>(null);
const volume = ref(1);
const precise = ref(false);
const notice = ref('');
const noticeKind = ref<'error' | 'success' | 'info'>('info');
const initializing = ref(false);
const savingConfig = ref(false);
const submitting = ref(false);
const inspectingPath = ref<string | null>(null);
const cancellingJobs = ref<Set<string>>(new Set());

const avReady = computed(
  () => Boolean(environment.value?.ffmpeg.available && environment.value?.ffprobe.available)
);
const environmentProbes = computed(() => {
  if (!environment.value) {
    return [
      { name: 'FFmpeg', available: false, resolved_path: null, version_summary: null, install_hint: null },
      { name: 'FFprobe', available: false, resolved_path: null, version_summary: null, install_hint: null },
    ];
  }
  return [
    { name: 'FFmpeg', ...environment.value.ffmpeg },
    { name: 'FFprobe', ...environment.value.ffprobe },
  ];
});
const currentAvFormats = computed(() => (avOperation.value === 'convert' ? convertFormats : audioFormats));
const isTrim = computed(() => avOperation.value === 'trim');
const isAdjustVolume = computed(() => avOperation.value === 'adjust_volume');

let pollTimer: ReturnType<typeof setInterval> | undefined;
let unlistenDragDrop: (() => void) | undefined;
let unlistenJobProgress: (() => void) | undefined;
let jobsRequestPending = false;
let disposed = false;

watch(avOperation, (operation) => {
  const formats = operation === 'convert' ? convertFormats : audioFormats;
  if (!formats.includes(avOutputFormat.value)) avOutputFormat.value = formats[0];
});

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function showNotice(message: string, kind: 'error' | 'success' | 'info' = 'info') {
  notice.value = message;
  noticeKind.value = kind;
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

function isImageFile(path: string): boolean {
  return /\.(png|jpe?g|webp|bmp|ico)$/i.test(path);
}

function addFiles(paths: string[]) {
  const unique = new Set(pendingFiles.value);
  paths.filter(Boolean).forEach((path) => unique.add(path));
  pendingFiles.value = Array.from(unique);
}

function removeFile(index: number) {
  pendingFiles.value.splice(index, 1);
}

function clearFiles() {
  pendingFiles.value = [];
}

async function pickFiles() {
  try {
    const paths = await invokeIpc<string[]>('media_pick_files');
    addFiles(paths || []);
  } catch (error) {
    showNotice(t('media.errors.pickFiles', { message: errorMessage(error) }), 'error');
  }
}

async function pickOutputDirectory() {
  try {
    const path = await invokeIpc<string | null>('media_pick_directory');
    if (path) outputDir.value = path;
  } catch (error) {
    showNotice(t('media.errors.pickDirectory', { message: errorMessage(error) }), 'error');
  }
}

async function loadEnvironment() {
  try {
    environment.value = await invokeIpc<MediaEnvironment>('media_check_ffmpeg_env');
  } catch (error) {
    environment.value = null;
    showNotice(t('media.errors.environment', { message: errorMessage(error) }), 'error');
  }
}

async function loadConfig() {
  try {
    const config = await invokeIpc<MediaConfig>('media_get_config');
    outputDir.value = config.output_dir || '';
    maxConcurrentJobs.value = config.max_concurrent_jobs;
  } catch (error) {
    showNotice(t('media.errors.loadConfig', { message: errorMessage(error) }), 'error');
  }
}

async function saveConfig() {
  const concurrency = Number(maxConcurrentJobs.value);
  if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 4) {
    showNotice(t('media.errors.invalidConcurrency'), 'error');
    return;
  }
  savingConfig.value = true;
  try {
    const config = await invokeIpc<MediaConfig>('media_update_config', {
      output_dir: outputDir.value.trim() || null,
      max_concurrent_jobs: concurrency,
    });
    outputDir.value = config.output_dir || '';
    maxConcurrentJobs.value = config.max_concurrent_jobs;
    showNotice(t('media.output.saved'), 'success');
  } catch (error) {
    showNotice(t('media.errors.saveConfig', { message: errorMessage(error) }), 'error');
  } finally {
    savingConfig.value = false;
  }
}

function mergeJobs(incoming: MediaJob[]) {
  jobs.value = [...incoming].sort((a, b) => b.created_at - a.created_at);
}

async function refreshJobs(reportError = false) {
  if (jobsRequestPending || disposed) return;
  jobsRequestPending = true;
  try {
    const result = await invokeIpc<MediaJob[]>('media_list_jobs');
    if (!disposed) mergeJobs(result || []);
  } catch (error) {
    if (reportError && !disposed) {
      showNotice(t('media.errors.loadJobs', { message: errorMessage(error) }), 'error');
    }
  } finally {
    jobsRequestPending = false;
  }
}

function optionalNumber(value: number | null): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function validateOptions(): boolean {
  if (mode.value === 'image') {
    const width = optionalNumber(imageWidth.value);
    const height = optionalNumber(imageHeight.value);
    if ((width !== null && (!Number.isInteger(width) || width <= 0)) ||
        (height !== null && (!Number.isInteger(height) || height <= 0))) {
      showNotice(t('media.errors.invalidDimensions'), 'error');
      return false;
    }
    if (!preserveAspect.value && (width === null || height === null)) {
      showNotice(t('media.errors.invalidDimensions'), 'error');
      return false;
    }
    if (imageFormat.value === 'jpeg' &&
        (!Number.isFinite(imageQuality.value) || imageQuality.value < 1 || imageQuality.value > 100)) {
      showNotice(t('media.errors.invalidQuality'), 'error');
      return false;
    }
  } else if (avOperation.value === 'trim') {
    const start = optionalNumber(startSeconds.value) ?? 0;
    const end = optionalNumber(endSeconds.value);
    if (start < 0 || end === null || end <= start) {
      showNotice(t('media.errors.invalidTrimRange'), 'error');
      return false;
    }
  } else if (avOperation.value === 'adjust_volume' &&
      (!Number.isFinite(volume.value) || volume.value < 0 || volume.value > 4)) {
    showNotice(t('media.errors.invalidVolume'), 'error');
    return false;
  }
  return true;
}

async function submitBatch() {
  const paths = [...pendingFiles.value];
  if (!paths.length || submitting.value || (mode.value === 'av' && !avReady.value) || !validateOptions()) return;
  submitting.value = true;
  let succeeded = 0;
  let failed = 0;
  try {
    if (mode.value === 'image') {
      try {
        const created = await invokeIpc<MediaJob[]>('media_convert_images', {
          input_paths: paths,
          output_dir: outputDir.value.trim() || null,
          options: {
            output_format: imageFormat.value,
            quality: imageFormat.value === 'jpeg' ? imageQuality.value : null,
            width: optionalNumber(imageWidth.value),
            height: optionalNumber(imageHeight.value),
            preserve_aspect: preserveAspect.value,
          },
        });
        succeeded = paths.length;
        pendingFiles.value = pendingFiles.value.filter((path) => !paths.includes(path));
        mergeJobs([...created, ...jobs.value]);
      } catch (error) {
        failed = paths.length;
        showNotice(t('media.errors.submit', { message: errorMessage(error) }), 'error');
      }
    } else {
      const results = await Promise.all(paths.map(async (path) => {
        try {
          const job = await invokeIpc<MediaJob>('media_submit_av_job', {
            input_path: path,
            operation: avOperation.value,
            output_format: avOperation.value === 'trim' ? null : avOutputFormat.value,
            output_dir: outputDir.value.trim() || null,
            start_seconds: avOperation.value === 'trim' ? optionalNumber(startSeconds.value) ?? 0 : null,
            end_seconds: avOperation.value === 'trim' ? optionalNumber(endSeconds.value) : null,
            volume: avOperation.value === 'adjust_volume' ? volume.value : null,
            precise: avOperation.value === 'trim' ? precise.value : false,
          });
          return { path, job, error: null };
        } catch (error) {
          return { path, job: null, error: errorMessage(error) };
        }
      }));
      const completed = results.filter((result) => result.job !== null);
      const rejected = results.filter((result) => result.job === null);
      succeeded = completed.length;
      failed = rejected.length;
      const succeededPaths = new Set(completed.map((result) => result.path));
      pendingFiles.value = pendingFiles.value.filter((path) => !succeededPaths.has(path));
      mergeJobs([...completed.map((result) => result.job as MediaJob), ...jobs.value]);
      if (rejected.length) {
        showNotice(t('media.errors.submit', {
          message: rejected[0].error || t('media.errors.unknown'),
        }), 'error');
      }
    }
    if (succeeded) {
      showNotice(t('media.jobs.submitted', { succeeded, failed }), failed ? 'info' : 'success');
    }
    await refreshJobs();
  } finally {
    submitting.value = false;
  }
}

async function inspectFile(path: string) {
  if ((!isImageFile(path) && !environment.value?.ffprobe.available) || inspectingPath.value) return;
  inspectingPath.value = path;
  try {
    const job = await invokeIpc<MediaJob>('media_inspect_file', { input_path: path });
    mergeJobs([job, ...jobs.value.filter((item) => item.id !== job.id)]);
    showNotice(t('media.inspect.started'), 'info');
    await refreshJobs();
  } catch (error) {
    showNotice(t('media.errors.inspect', { message: errorMessage(error) }), 'error');
  } finally {
    inspectingPath.value = null;
  }
}

async function cancelJob(jobId: string) {
  const next = new Set(cancellingJobs.value);
  next.add(jobId);
  cancellingJobs.value = next;
  try {
    const job = await invokeIpc<MediaJob | null>('media_cancel_job', { job_id: jobId });
    if (job) mergeJobs(jobs.value.map((item) => (item.id === job.id ? job : item)));
    else await refreshJobs();
  } catch (error) {
    showNotice(t('media.errors.cancel', { message: errorMessage(error) }), 'error');
  } finally {
    const remaining = new Set(cancellingJobs.value);
    remaining.delete(jobId);
    cancellingJobs.value = remaining;
  }
}

function normalizedProgress(progress: number): number {
  return Math.round(Math.min(100, Math.max(0, Number.isFinite(progress) ? progress : 0)));
}

function operationLabel(operation: string): string {
  switch (operation) {
    case 'convert': return t('media.operations.convert');
    case 'trim': return t('media.operations.trim');
    case 'extract_audio': return t('media.operations.extractAudio');
    case 'adjust_volume': return t('media.operations.adjustVolume');
    case 'image_convert':
    case 'convert_image': return t('media.operations.imageConvert');
    case 'inspect':
    case 'probe': return t('media.operations.inspect');
    default: return t('media.operations.generic');
  }
}

function statusLabel(status: JobStatus): string {
  switch (status) {
    case 'queued': return t('media.status.queued');
    case 'running': return t('media.status.running');
    case 'completed': return t('media.status.completed');
    case 'failed': return t('media.status.failed');
    case 'cancelled': return t('media.status.cancelled');
  }
}

function formatMetadata(metadata: any): string {
  try {
    return JSON.stringify(metadata, null, 2);
  } catch {
    return String(metadata);
  }
}

function metadataSummary(job: MediaJob): string[] {
  const metadata = job.metadata;
  if (!metadata || typeof metadata !== 'object') return [];
  const summary: string[] = [];
  const format = metadata.format;
  const streams = Array.isArray(metadata.streams) ? metadata.streams : [];
  if (format?.format_name) {
    summary.push(t('media.inspect.format', { value: format.format_name }));
  }
  const duration = Number(format?.duration);
  if (Number.isFinite(duration)) {
    summary.push(t('media.inspect.duration', { seconds: duration.toFixed(2) }));
  }
  const size = Number(format?.size);
  if (Number.isFinite(size)) {
    summary.push(t('media.inspect.size', { value: (size / 1024 / 1024).toFixed(2) }));
  }
  const video = streams.find((stream: any) => stream.codec_type === 'video');
  if (video) {
    summary.push(t('media.inspect.video', {
      codec: video.codec_name || '-',
      width: video.width || 0,
      height: video.height || 0,
    }));
  }
  const audio = streams.find((stream: any) => stream.codec_type === 'audio');
  if (audio) {
    summary.push(t('media.inspect.audio', {
      codec: audio.codec_name || '-',
      channels: audio.channels || 0,
    }));
  }
  return summary;
}

function formatDateTime(timestamp: number): string {
  const date = new Date(timestamp < 1_000_000_000_000 ? timestamp * 1000 : timestamp);
  return date.toLocaleString();
}

async function registerTauriListeners() {
  const eventApi = (window as any).__TAURI__?.event;
  if (typeof eventApi?.listen !== 'function') return;
  try {
    const unlisten = await eventApi.listen('tauri://drag-drop', (event: { payload: DragDropPayload }) => {
      addFiles(event.payload?.paths || []);
    });
    if (disposed) unlisten();
    else unlistenDragDrop = unlisten;
  } catch (error) {
    console.error('Media drag-drop listener failed:', error);
  }
  try {
    const unlisten = await eventApi.listen('media://job-progress', (event: { payload: JobProgressPayload }) => {
      const update = event.payload;
      if (update.job) {
        mergeJobs([update.job, ...jobs.value.filter((job) => job.id !== update.job_id)]);
        return;
      }
      const target = jobs.value.find((job) => job.id === update.job_id);
      if (target) {
        target.progress = update.progress;
        target.speed = update.speed;
        if (update.progress >= 100) void refreshJobs();
      } else {
        void refreshJobs();
      }
    });
    if (disposed) unlisten();
    else unlistenJobProgress = unlisten;
  } catch (error) {
    console.error('Media progress listener failed:', error);
  }
}

async function initialize() {
  if (initializing.value) return;
  initializing.value = true;
  try {
    await Promise.all([loadEnvironment(), loadConfig(), refreshJobs(true)]);
  } finally {
    initializing.value = false;
  }
}

onMounted(async () => {
  await initialize();
  await registerTauriListeners();
  if (!disposed) pollTimer = setInterval(() => void refreshJobs(), 1500);
});

onUnmounted(() => {
  disposed = true;
  if (pollTimer) clearInterval(pollTimer);
  unlistenDragDrop?.();
  unlistenJobProgress?.();
});
</script>

<style scoped>
.media-panel {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 14px;
  color: var(--macos-text-primary);
}

.media-heading,
.card-heading,
.job-topline,
.job-actions,
.button-row,
.input-action-row,
.probe-title-row,
.progress-row {
  display: flex;
  align-items: center;
}

.media-heading,
.card-heading,
.job-topline {
  justify-content: space-between;
  gap: 12px;
}

.media-heading h2 {
  font-size: 18px;
  line-height: 1.3;
  font-weight: 650;
  letter-spacing: -0.02em;
}

.media-heading p,
.card-heading p {
  margin-top: 3px;
  color: var(--macos-text-tertiary);
  font-size: 11px;
}

.media-card {
  padding: 14px;
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-md);
  background: var(--macos-bg-card);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.025);
}

.card-heading {
  margin-bottom: 12px;
}

.card-heading.compact {
  align-items: flex-start;
}

.card-heading h3 {
  font-size: 13px;
  font-weight: 600;
}

.media-notice {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 9px 11px;
  border: 1px solid rgba(var(--macos-accent-rgb), 0.2);
  border-radius: var(--macos-radius-sm);
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
  font-size: 11px;
  user-select: text;
}

.media-notice.error {
  border-color: rgba(255, 59, 48, 0.2);
  background: var(--macos-danger-subtle);
  color: var(--macos-danger);
}

.media-notice.success {
  border-color: rgba(52, 199, 89, 0.22);
  background: var(--macos-success-subtle);
  color: var(--macos-success);
}

.notice-close,
.icon-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: inherit;
  cursor: pointer;
}

.notice-close {
  width: 20px;
  height: 20px;
  margin: -2px -3px -2px 0;
}

.notice-close:hover,
.icon-button:hover {
  background: rgba(128, 128, 128, 0.13);
}

.availability-badge,
.job-count,
.status-badge,
.operation-badge {
  flex-shrink: 0;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 500;
}

.availability-badge,
.job-count {
  padding: 2px 8px;
  background: var(--macos-danger-subtle);
  color: var(--macos-danger);
}

.availability-badge.ready {
  background: var(--macos-success-subtle);
  color: var(--macos-success);
}

.probe-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.probe-item {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  min-width: 0;
  padding: 9px 10px;
  border: 1px solid var(--macos-border-separator);
  border-radius: var(--macos-radius-sm);
  background: var(--macos-bg-input);
}

.probe-dot {
  width: 7px;
  height: 7px;
  margin-top: 5px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--macos-danger);
}

.probe-dot.ready {
  background: var(--macos-success);
  box-shadow: 0 0 0 3px var(--macos-success-subtle);
}

.probe-content {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  gap: 2px;
}

.probe-title-row {
  justify-content: space-between;
  gap: 8px;
  font-size: 11px;
}

.probe-title-row strong {
  font-size: 12px;
}

.probe-title-row span,
.probe-detail {
  color: var(--macos-text-tertiary);
  font-size: 10px;
}

.probe-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.probe-hint,
.environment-warning,
.option-note {
  color: var(--macos-text-secondary);
  font-size: 10px;
  line-height: 1.5;
}

.probe-hint {
  margin-top: 4px;
}

.environment-warning {
  margin-top: 9px;
  color: var(--macos-danger);
}

.output-settings {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 130px;
  align-items: end;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 5px;
  color: var(--macos-text-secondary);
  font-size: 11px;
}

.input-action-row {
  gap: 7px;
}

.input-action-row .media-input {
  flex: 1;
}

.media-input {
  width: 100%;
  min-width: 0;
  height: 30px;
  padding: 5px 8px;
  border: 1px solid var(--macos-border-subtle);
  border-radius: var(--macos-radius-sm);
  outline: none;
  background: var(--macos-bg-input);
  color: var(--macos-text-primary);
  font: inherit;
  font-size: 11px;
  user-select: text;
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

.media-input:focus {
  border-color: var(--macos-border-focus);
  box-shadow: 0 0 0 3px var(--macos-accent-subtle);
}

select.media-input {
  cursor: pointer;
}

.media-workspace {
  display: grid;
  grid-template-columns: minmax(0, 1.12fr) minmax(290px, 0.88fr);
  align-items: start;
  gap: 14px;
}

.source-card,
.options-card {
  min-width: 0;
}

.button-row,
.file-actions,
.job-actions {
  gap: 6px;
}

.file-list {
  display: flex;
  flex-direction: column;
  max-height: 330px;
  min-height: 206px;
  overflow-y: auto;
  border: 1px dashed var(--macos-border-subtle);
  border-radius: var(--macos-radius-sm);
  background: var(--macos-bg-input);
}

.file-list.empty {
  align-items: center;
  justify-content: center;
}

.empty-files {
  display: flex;
  align-items: center;
  flex-direction: column;
  gap: 6px;
  padding: 30px 16px;
  color: var(--macos-text-tertiary);
  text-align: center;
}

.empty-files strong {
  color: var(--macos-text-secondary);
  font-size: 12px;
  font-weight: 500;
}

.empty-files span {
  max-width: 300px;
  font-size: 10px;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  padding: 8px 9px;
  border-bottom: 1px solid var(--macos-border-separator);
}

.file-row:last-child {
  border-bottom: 0;
}

.file-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: 6px;
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
}

.file-info {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.file-info strong,
.file-info span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-info strong {
  font-size: 11px;
  font-weight: 500;
}

.file-info span {
  color: var(--macos-text-tertiary);
  font-size: 9px;
  user-select: text;
}

.icon-button {
  width: 25px;
  height: 25px;
  color: var(--macos-text-tertiary);
}

.small-button {
  min-height: 25px;
  padding: 3px 8px;
  border: 1px solid var(--macos-border-subtle);
  border-radius: 5px;
  background: var(--macos-bg-card);
  color: var(--macos-text-secondary);
  font-size: 10px;
  cursor: pointer;
  white-space: nowrap;
}

.small-button:hover {
  border-color: var(--macos-border-focus);
  color: var(--macos-accent);
}

.small-button.danger:hover {
  border-color: var(--macos-danger);
  color: var(--macos-danger);
}

.mode-switch {
  display: flex;
  margin-bottom: 13px;
}

.mode-switch .macos-segment-item {
  flex: 1;
  justify-content: center;
}

.options-form {
  display: flex;
  flex-direction: column;
  min-height: 162px;
  gap: 11px;
}

.dimension-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 9px;
}

.quality-controls {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 66px;
  align-items: center;
  gap: 10px;
}

.quality-controls input[type='range'] {
  width: 100%;
  accent-color: var(--macos-accent);
}

.checkbox-field {
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--macos-text-secondary);
  font-size: 11px;
  cursor: pointer;
}

.checkbox-field input {
  accent-color: var(--macos-accent);
}

.option-note {
  padding: 7px 8px;
  border-radius: 5px;
  background: var(--macos-accent-subtle);
}

.option-note.warning {
  background: var(--macos-danger-subtle);
  color: var(--macos-danger);
}

.submit-button {
  width: 100%;
  margin-top: 13px;
}

.jobs-card {
  min-width: 0;
}

.job-count {
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
}

.empty-jobs {
  padding: 28px 12px;
  color: var(--macos-text-tertiary);
  font-size: 11px;
  text-align: center;
}

.job-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.job-item {
  min-width: 0;
  padding: 10px;
  border: 1px solid var(--macos-border-separator);
  border-radius: var(--macos-radius-sm);
  background: var(--macos-bg-input);
}

.job-title {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 8px;
}

.job-title strong {
  overflow: hidden;
  font-size: 11px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.operation-badge {
  padding: 2px 7px;
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
}

.status-badge {
  padding: 2px 7px;
  background: rgba(128, 128, 128, 0.12);
  color: var(--macos-text-secondary);
}

.status-badge.running {
  background: var(--macos-accent-subtle);
  color: var(--macos-accent);
}

.status-badge.completed {
  background: var(--macos-success-subtle);
  color: var(--macos-success);
}

.status-badge.failed,
.status-badge.cancelled {
  background: var(--macos-danger-subtle);
  color: var(--macos-danger);
}

.job-path {
  margin: 5px 0 8px;
  overflow: hidden;
  color: var(--macos-text-tertiary);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 9px;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.progress-row {
  gap: 8px;
}

.progress-track {
  height: 5px;
  flex: 1;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(128, 128, 128, 0.16);
}

.progress-value {
  height: 100%;
  border-radius: inherit;
  background: var(--macos-accent);
  transition: width 0.25s ease;
}

.progress-value.completed {
  background: var(--macos-success);
}

.progress-value.failed,
.progress-value.cancelled {
  background: var(--macos-danger);
}

.progress-number,
.job-speed,
.job-time {
  color: var(--macos-text-tertiary);
  font-size: 9px;
}

.progress-number {
  width: 30px;
  text-align: right;
}

.job-speed {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.job-error {
  margin-top: 7px;
  color: var(--macos-danger);
  font-size: 10px;
  user-select: text;
}

.job-output {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 8px;
  margin-top: 7px;
  color: var(--macos-text-secondary);
  font-size: 10px;
}

.job-output code {
  overflow: hidden;
  color: var(--macos-text-tertiary);
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.metadata-summary {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 8px;
}

.metadata-summary span {
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--macos-accent-subtle);
  color: var(--macos-text-secondary);
  font-size: 9px;
}

.metadata-details {
  margin-top: 8px;
  color: var(--macos-text-secondary);
  font-size: 10px;
}

.metadata-details summary {
  width: fit-content;
  cursor: pointer;
}

.metadata-details pre {
  max-height: 300px;
  margin-top: 7px;
  padding: 9px;
  overflow: auto;
  border: 1px solid var(--macos-border-separator);
  border-radius: 5px;
  background: var(--macos-bg-card-active);
  color: var(--macos-text-secondary);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 9px;
  line-height: 1.5;
  user-select: text;
}

.job-time {
  margin-top: 7px;
  text-align: right;
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.45;
  transform: none;
}

button:focus-visible,
summary:focus-visible {
  outline: 2px solid var(--macos-border-focus);
  outline-offset: 2px;
}

@media (max-width: 860px) {
  .media-workspace {
    grid-template-columns: 1fr;
  }

  .options-form {
    min-height: 0;
  }

  .file-list {
    max-height: 280px;
  }
}

@media (max-width: 620px) {
  .media-heading,
  .card-heading {
    align-items: flex-start;
  }

  .probe-grid,
  .output-settings {
    grid-template-columns: 1fr;
  }

  .concurrency-field {
    max-width: 160px;
  }

  .source-card .card-heading {
    flex-direction: column;
  }

  .button-row {
    width: 100%;
  }

  .button-row .macos-button {
    flex: 1;
  }
}

@media (max-width: 440px) {
  .media-card {
    padding: 11px;
  }

  .media-heading {
    align-items: flex-start;
    flex-direction: column;
  }

  .file-row {
    align-items: flex-start;
    flex-wrap: wrap;
  }

  .file-info {
    width: calc(100% - 40px);
    flex: auto;
  }

  .file-actions {
    width: 100%;
    justify-content: flex-end;
  }

  .input-action-row {
    align-items: stretch;
    flex-direction: column;
  }

  .job-topline {
    align-items: flex-start;
    flex-direction: column;
  }

  .job-actions {
    width: 100%;
    justify-content: flex-end;
  }
}
</style>
