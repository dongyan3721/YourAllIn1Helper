export const FEATURES = [
  { module: 'screenshot', tab: 'screenshot', icon: 'camera' },
  { module: 'clipboard', tab: 'clipboard', icon: 'clipboard' },
  { module: 'screen_ink', tab: 'screen_ink', icon: 'brush' },
  { module: 'sticky_note', tab: 'sticky', icon: 'sticky' },
  { module: 'app_usage', tab: 'usage', icon: 'chart' },
  { module: 'script_runner', tab: 'scripts', icon: 'terminal' },
  { module: 'translator', tab: 'translator', icon: 'translate' },
  { module: 'media_proc', tab: 'media', icon: 'photo' },
  { module: 'window_pin', tab: 'winpin', icon: 'pin' },
  { module: 'password_vault', tab: 'vault', icon: 'lock' },
] as const;
export function featureForModule(id: string) { return FEATURES.find(feature => feature.module === id); }
export function featureForRoute(route: string) {
  const name = route.replace(/^\/?dashboard\//, '').replace(/^\/?feature\//, '').replace(/^\//, '');
  return FEATURES.find(feature => feature.tab === name || feature.module === name);
}
/** Record only key combinations accepted by the native shortcut parser. */
export function shortcutFromEvent(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'isComposing' | 'repeat'>): string | null {
  if (event.isComposing || event.repeat || ['Control', 'Alt', 'Shift', 'Meta', 'Escape', 'Tab'].includes(event.key)) return null;
  let key = '';
  if (/^Key[A-Z]$/.test(event.code)) key = event.code.slice(3);
  else if (/^Digit[0-9]$/.test(event.code)) key = event.code.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(event.key)) key = event.key;
  else {
    const keys: Record<string, string> = { Space: 'Space', Enter: 'Enter', Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert', Home: 'Home', End: 'End', PageUp: 'Pageup', PageDown: 'Pagedown', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Backquote: 'Grave', Minus: 'Minus', Equal: 'Equal' };
    key = keys[event.code] || '';
  }
  if (!key || (!(event.ctrlKey || event.altKey || event.metaKey) && !/^F\d+$/.test(key))) return null;
  return [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift', event.metaKey && 'Super', key].filter(Boolean).join('+');
}
export function displayRecordedShortcut(hotkey: string, os: string): string {
  const mac: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' };
  return hotkey.split('+').map(token => os === 'macos' ? mac[token] || token : token === 'Super' ? 'Win' : token).join(' + ');
}
