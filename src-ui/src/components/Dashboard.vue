<template>
  <div class="launcher macos-window">
    <header class="launcher-header">
      <div class="brand"><ProductLogo :size="27" /><h1>{{ t('app.name') }}</h1></div>
      <nav class="launcher-tabs" role="tablist" :aria-label="t('desktop.navigation')">
        <button id="features-tab" role="tab" :aria-selected="page === 'features'" aria-controls="features-page" :class="{ active: page === 'features' }" @click="page = 'features'">{{ t('desktop.features') }}</button>
        <button id="preferences-tab" role="tab" :aria-selected="page === 'preferences'" aria-controls="preferences-page" :class="{ active: page === 'preferences' }" @click="page = 'preferences'">{{ t('desktop.preferences') }}</button>
      </nav>
    </header>
    <main class="launcher-content">
      <p v-if="error" class="launcher-error" role="alert">{{ error }}</p>
      <section v-if="page === 'features'" id="features-page" role="tabpanel" aria-labelledby="features-tab">
        <div class="page-heading"><h2>{{ t('desktop.yourTools') }}</h2><p>{{ t('desktop.featuresHint') }}</p></div>
        <p v-if="!status" class="loading-text" role="status">{{ t('desktop.loading') }}</p>
        <ModuleManager v-else :modules="status.module_catalog" :opening="opening" @changed="refreshDashboard" @open="openFeature" />
      </section>
      <section v-else id="preferences-page" role="tabpanel" aria-labelledby="preferences-tab"><PreferencesPanel :status="status" @changed="refreshDashboard" /></section>
    </main>
    <footer class="launcher-footer">
      <button class="footer-link" @click="openManual">{{ t('desktop.manual') }} <span aria-hidden="true">↗</span></button>
      <button class="footer-link quit" @click="shutdownDaemon">{{ t('app.quit') }}</button>
    </footer>
  </div>
</template>
<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import { invokeIpc, type KernelStatus } from '../api/ipc';
import { t, syncLocaleFromBackendConfig } from '../utils/i18n';
import { featureForRoute } from '../utils/features';
import ProductLogo from './ProductLogo.vue';
import ModuleManager from './ModuleManager.vue';
import PreferencesPanel from './PreferencesPanel.vue';
const page = ref('features');
const status = ref<KernelStatus | null>(null);
const opening = ref<string | null>(null);
const error = ref('');
let unlistenNavigate: (() => void) | undefined;
let unlistenKernel: (() => void) | undefined;
let refreshTimer: ReturnType<typeof setInterval> | undefined;
let refreshing = false;
async function refreshDashboard() {
  if (refreshing) return;
  refreshing = true;
  try {
    status.value = await invokeIpc<KernelStatus>('kernel_get_status');
    if (status.value.config.general?.language) syncLocaleFromBackendConfig(status.value.config.general.language);
    error.value = '';
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { refreshing = false; }
}
async function openFeature(moduleId: string) {
  opening.value = moduleId; error.value = '';
  try { await invokeIpc('kernel_open_feature', { module_id: moduleId }); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { opening.value = null; }
}
async function applyDashboardRoute(route: string | null) {
  if (!route) return;
  const normalized = route.replace(/^\/?dashboard\//, '').replace(/^\//, '');
  if (normalized === 'preferences') page.value = 'preferences';
  else if (['overview', 'features', ''].includes(normalized)) page.value = 'features';
  else { const feature = featureForRoute(route); if (feature) await openFeature(feature.module); }
}
async function openManual() {
  try { await invokeIpc('kernel_open_manual'); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}
async function shutdownDaemon() {
  try { await invokeIpc('kernel_shutdown'); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}
onMounted(async () => {
  await refreshDashboard();
  await applyDashboardRoute(new URLSearchParams(window.location.search).get('route'));
  const eventApi = (window as any).__TAURI__?.event;
  if (typeof eventApi?.listen === 'function') {
    unlistenNavigate = await eventApi.listen('dashboard://navigate', (event: { payload: string }) => void applyDashboardRoute(event.payload));
    unlistenKernel = await eventApi.listen('kernel://updated', () => void refreshDashboard());
  }
  refreshTimer = setInterval(refreshDashboard, 5000);
});
onUnmounted(() => { if (refreshTimer) clearInterval(refreshTimer); unlistenNavigate?.(); unlistenKernel?.(); });
</script>
<style scoped>
.launcher { display: flex; flex-direction: column; height: 100vh; background: var(--macos-bg-window); }
.launcher-header { padding: 22px 28px 0; flex: 0 0 auto; }
.brand { display: flex; gap: 11px; align-items: center; }
.brand h1 { font-size: 17px; font-weight: 650; letter-spacing: -.4px; }
.launcher-tabs { display: flex; gap: 26px; border-bottom: 1px solid var(--macos-border-subtle); margin-top: 22px; }
.launcher-tabs button { background: none; color: var(--macos-text-secondary); border: 0; padding: 0 1px 12px; font-size: 13px; cursor: pointer; position: relative; }
.launcher-tabs button.active { color: var(--macos-text-primary); font-weight: 600; }
.launcher-tabs button.active::after { content: ''; height: 2px; background: var(--macos-accent); position: absolute; bottom: -1px; left: 0; right: 0; border-radius: 2px; }
.launcher-content { padding: 22px 28px 14px; flex: 1; min-height: 0; overflow: auto; }
.page-heading { margin-bottom: 18px; }
.page-heading h2 { font-size: 14px; font-weight: 600; }
.page-heading p { font-size: 12px; color: var(--macos-text-secondary); margin-top: 5px; }
.launcher-footer { display: flex; justify-content: space-between; padding: 13px 28px 17px; border-top: 1px solid var(--macos-border-subtle); }
.footer-link { background: none; border: 0; padding: 2px 0; color: var(--macos-text-secondary); font-size: 12px; cursor: pointer; }
.footer-link:hover { color: var(--macos-accent); }
.quit:hover { color: var(--macos-danger); }
.launcher-error { color: var(--macos-danger); font-size: 12px; margin-bottom: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
.loading-text { color: var(--macos-text-secondary); padding: 30px 0; }
button:focus-visible { outline: 2px solid var(--macos-accent); outline-offset: 5px; }
@media (max-width: 500px) { .launcher-header { padding: 18px 18px 0; } .launcher-content { padding: 18px; } .launcher-footer { padding: 12px 18px; } }
</style>
