import { ref } from 'vue';
export const toastMessage = ref<{ text: string; error: boolean } | null>(null);
let timer: ReturnType<typeof setTimeout> | undefined;
export function notify(text: string, error = false) {
  if (timer) clearTimeout(timer);
  toastMessage.value = { text, error };
  timer = setTimeout(() => { toastMessage.value = null; }, error ? 7000 : 6000);
}
