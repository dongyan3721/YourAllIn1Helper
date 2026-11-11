import { ref, onMounted, onUnmounted } from 'vue';
import { invokeIpc, type ShortcutPreference } from '../api/ipc';

/** Read configured bindings, including unassigned actions; never invent defaults in UI. */
export function useActionShortcuts() {
  const shortcuts = ref<ShortcutPreference[]>([]);
  let disposed = false;
  let revision = 0;
  let unlisten: (() => void) | undefined;
  async function refresh() {
    const request = ++revision;
    try {
      const result = await invokeIpc<{ shortcuts: ShortcutPreference[] }>('kernel_get_shortcut_preferences');
      if (!disposed && request === revision) shortcuts.value = result.shortcuts;
    } catch { /* A missing binding is shown without a keyboard hint. */ }
  }
  onMounted(async () => {
    void refresh();
    window.addEventListener('focus', refresh);
    const events = (window as any).__TAURI__?.event;
    if (events?.listen) {
      unlisten = await events.listen('kernel://updated', refresh);
      if (disposed) unlisten?.();
    }
  });
  onUnmounted(() => { disposed = true; ++revision; unlisten?.(); window.removeEventListener('focus', refresh); });
  function caption(text: string, action: string) {
    const label = shortcuts.value.find(row => row.spec.action_id === action)?.display_label;
    return label ? `${text} (${label})` : text;
  }
  return { caption };
}

export function localShortcut(keys: string): string {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/i.test(navigator.platform);
  return keys.replace(/PrimaryMod/g, mac ? '⌘' : 'Ctrl');
}
