/**
 * GIF 录制前端格式化与预设工具。
 */

export function formatGifDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
}

export function formatFileSize(bytes: number): string {
  if (bytes <= 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export const GIF_FPS_OPTIONS = [10, 15, 24, 30] as const;

export const GIF_RESOLUTION_PRESETS = [
  { id: 'original', labelKey: 'screenshot.gif.presetOriginal' },
  { id: '1080p', labelKey: 'screenshot.gif.preset1080p' },
  { id: '720p', labelKey: 'screenshot.gif.preset720p' },
  { id: '480p', labelKey: 'screenshot.gif.preset480p' },
] as const;
