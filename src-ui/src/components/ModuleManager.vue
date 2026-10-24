<template>
  <div class="section-card">
    <div class="card-header-bar">
      <span class="card-title">功能积木</span>
      <span class="card-desc">开关即时生效并保存；关闭功能保留已有数据</span>
    </div>
    <p v-if="error" class="module-error" role="alert">{{ error }}</p>
    <div class="table-container">
      <table class="macos-table">
        <thead><tr><th>序号</th><th>功能</th><th>状态</th><th>开关</th></tr></thead>
        <tbody>
          <tr v-for="module in modules" :key="module.id">
            <td>{{ String(module.number).padStart(2, '0') }}</td>
            <td>{{ module.name }}</td>
            <td>{{ module.unavailable_reason || (module.enabled ? '已开启' : '已关闭') }}</td>
            <td>
              <button class="macos-button" :class="{ primary: module.enabled }"
                role="switch" :aria-checked="module.enabled" :aria-label="`${module.name}开关`"
                :disabled="!module.available || pending !== null"
                @click="toggle(module)">
                {{ !module.available ? '待开发' : pending === module.id ? '处理中…' : module.enabled ? '关闭' : '开启' }}
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
defineProps<{ modules: ModuleStatus[] }>();
const emit = defineEmits<{ changed: [] }>();
const pending = ref<string | null>(null);
const error = ref('');
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
