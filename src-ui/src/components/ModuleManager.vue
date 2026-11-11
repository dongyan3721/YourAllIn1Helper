<template>
  <div>
    <p v-if="error" class="module-error" role="alert">{{ error }}</p>
    <div class="feature-grid">
      <article v-for="module in modules" :key="module.id" class="feature-row" :class="{ inactive: !module.enabled }">
        <div class="feature-icon"><Icons :name="FEATURES.find(f => f.module === module.id)?.icon || 'dashboard'" :size="20" /></div>
        <div class="feature-info">
          <h3>{{ formatModuleName(module) }}</h3>
          <button class="open-link" :aria-label="t('desktop.openFeature', { name: formatModuleName(module) })" :disabled="!module.enabled || !module.available || !module.supported || opening != null || pending !== null" :title="module.enabled ? '' : t('desktop.enableFirst')" @click="emit('open', module.id)">
            {{ opening === module.id ? t('desktop.opening') : t('desktop.openPanel') }} <span aria-hidden="true">↗</span>
          </button>
        </div>
        <button class="feature-switch" :class="{ enabled: module.enabled }" role="switch" :aria-checked="module.enabled" :aria-label="t('moduleManager.switchAria', { name: formatModuleName(module) })" :title="formatModuleStatus(module)" :disabled="!module.available || !module.supported || pending !== null" :aria-busy="pending === module.id" @click="toggle(module)"><span></span></button>
      </article>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { invokeIpc, type ModuleStatus } from '../api/ipc';
import Icons from './Icons.vue';
import { FEATURES } from '../utils/features';
import { t } from '../utils/i18n';

defineProps<{ modules: ModuleStatus[]; opening?: string | null }>();
const emit = defineEmits<{ changed: []; open: [moduleId: string] }>();
const pending = ref<string | null>(null);
const error = ref('');

/** 根据当前语言返回本地化模块名称 */
function formatModuleName(module: ModuleStatus): string {
  const key = `moduleManager.names.${module.id}`;
  const translated = t(key);
  return translated !== key ? translated : module.name;
}

/** 根据当前语言格式化模块运行/受限状态文案 */
function formatModuleStatus(module: ModuleStatus): string {
  if (!module.available) {
    return t('moduleManager.statusPendingDev');
  }
  if (!module.supported) {
    return t('moduleManager.statusUnsupportedWinPin');
  }
  if (module.unavailable_reason) {
    return module.unavailable_reason;
  }
  return module.enabled ? t('moduleManager.statusEnabled') : t('moduleManager.statusDisabled');
}

async function toggle(module: ModuleStatus) {
  pending.value = module.id;
  error.value = '';
  try {
    await invokeIpc('kernel_set_module_enabled', { module_id: module.id, enabled: !module.enabled });
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    emit('changed');
    pending.value = null;
  }
}
</script>

<style scoped>
.feature-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap: 30px; }
.feature-row { display: flex; align-items: center; gap: 13px; padding: 17px 0; border-bottom: 1px solid var(--macos-border-subtle); min-width: 0; }
.feature-icon { width: 38px; height: 38px; background: var(--macos-accent-subtle); color: var(--macos-accent); display: flex; align-items: center; justify-content: center; border-radius: 11px; flex: 0 0 auto; }
.inactive .feature-icon { background: var(--macos-bg-sidebar); color: var(--macos-text-tertiary); }
.feature-info { flex: 1; min-width: 0; }
h3 { font-size: 13px; font-weight: 600; margin: 0 0 4px; }
.open-link { padding: 0; border: 0; background: none; color: var(--macos-accent); font-size: 11px; cursor: pointer; }
.open-link:hover:not(:disabled) { text-decoration: underline; text-underline-offset: 3px; }
.open-link:disabled { color: var(--macos-text-tertiary); cursor: default; }
.feature-switch { width: 32px; height: 19px; border-radius: 12px; border: 1px solid var(--macos-border-subtle); background: var(--macos-scrollbar-thumb); padding: 2px; cursor: pointer; flex: 0 0 auto; transition: background .15s; }
.feature-switch span { display: block; width: 13px; height: 13px; background: white; box-shadow: 0 1px 3px #0002; border-radius: 50%; transition: transform .15s; }
.feature-switch.enabled { background: var(--macos-accent); border-color: var(--macos-accent); }
.feature-switch.enabled span { transform: translateX(13px); }
.feature-switch:disabled { opacity: .45; cursor: default; }
button:focus-visible { outline: 2px solid var(--macos-accent); outline-offset: 4px; }
.module-error { color: var(--macos-danger); font-size: 12px; margin-bottom: 10px; overflow-wrap: anywhere; }
@media (max-width: 540px) { .feature-grid { grid-template-columns: 1fr; } }
</style>
