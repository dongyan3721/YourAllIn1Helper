/** Frameless Tauri windows use the native drag API; form controls keep their own gestures. */
export async function startWindowDrag(event: MouseEvent): Promise<void> {
  if (event.button !== 0) return;
  const target = event.target as Element | null;
  if (target?.closest('button, input, textarea, select, a, [contenteditable], [data-no-drag]')) return;
  const current = (window as any).__TAURI__?.window?.getCurrentWindow?.();
  if (!current?.startDragging) return;
  event.preventDefault();
  try { await current.startDragging(); }
  catch (error) { console.error('Window drag failed:', error); }
}
