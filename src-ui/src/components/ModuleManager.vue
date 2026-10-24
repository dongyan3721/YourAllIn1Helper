<template>
  <div class="section-card">
    <div class="card-header-bar">
      <span class="card-title">{{ t('moduleManager.title') }}</span>
      <span class="card-desc">{{ t('moduleManager.desc') }}</span>
    </div>
    <p v-if="error" class="module-error" role="alert">{{ error }}</p>
    <div class="table-container">
      <table class="macos-table">
        <thead>
          <tr>
            <th>{{ t('moduleManager.colNumber') }}</th>
            <th>{{ t('moduleManager.colFeature') }}</th>
            <th>{{ t('moduleManager.colStatus') }}</th>
            <th>{{ t('moduleManager.colSwitch') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="module in modules" :key="module.id">
            <td>{{ String(module.number).padStart(2, '0') }}</td>
            <td>{{ formatModuleName(module) }}</td>
            <td>{{ formatModuleStatus(module) }}</td>
            <td>
              <button
                class="macos-button"
                :class="{ primary: module.enabled }"
                role="switch"
                :aria-checked="module.enabled"
                :aria-label="t('moduleManager.switchAria', { name: formatModuleName(module) })"
                :disabled="!module.available || pending !== null"
                @click="toggle(module)"
              >
                {{
                  !module.available
                    ? t('moduleManager.btnPendingDev')
                    : pending === module.id
                    ? t('moduleManager.btnProcessing')
                    : module.enabled
                    ? t('moduleManager.btnDisable')
                    : t('moduleManager.btnEnable')
                }}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { invokeIpc, type ModuleStatus } from '../api/ipc';
import { t } from '../utils/i18n';

defineProps<{ modules: ModuleStatus[] }>();
const emit = defineEmits<{ changed: [] }>();
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
/* 子组件独立维护标题布局，避免依赖父组件的 scoped 样式。 */
.card-header-bar {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px 16px;
}
.card-title { font-size: 13px; font-weight: 600; }
.card-desc { font-size: 11px; color: var(--macos-text-tertiary); }

.module-error { color: var(--macos-danger, #c53b36); padding: 0 16px; }
button:disabled { opacity: 0.5; cursor: default; }
</style>
