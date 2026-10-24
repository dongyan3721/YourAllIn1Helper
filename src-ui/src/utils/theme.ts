/**
 * macOS 原生主题与色谱管理系统
 * 严格遵循 Apple Human Interface Guidelines (HIG) 设计规范：
 * 1. 默认白蓝色调（macOS Light 经典风格）；
 * 2. 支持开灯/关灯/跟随系统三档模式切换；
 * 3. 支持 Apple 原生多重强调色切换（蓝、紫、粉、橙、绿、石墨灰）；
 * 4. 多窗口间通过 Storage 事件无感秒级同步。
 */

import { ref, watchEffect } from 'vue';

export type ThemeMode = 'light' | 'dark' | 'system';
export type AccentColor = 'blue' | 'purple' | 'pink' | 'orange' | 'green' | 'graphite';

export interface AccentColorSpec {
  id: AccentColor;
  name: string;
  lightHex: string;
  darkHex: string;
  rgb: string;
}

export const ACCENT_PALETTE: AccentColorSpec[] = [
  { id: 'blue', name: '经典蓝', lightHex: '#007aff', darkHex: '#0a84ff', rgb: '0, 122, 255' },
  { id: 'purple', name: '罗兰紫', lightHex: '#af52de', darkHex: '#bf5af2', rgb: '175, 82, 222' },
  { id: 'pink', name: '亮粉红', lightHex: '#ff2d55', darkHex: '#ff375f', rgb: '255, 45, 85' },
  { id: 'orange', name: '暖夕橙', lightHex: '#ff9500', darkHex: '#ff9f0a', rgb: '255, 149, 0' },
  { id: 'green', name: '薄荷绿', lightHex: '#34c759', darkHex: '#30d158', rgb: '52, 199, 89' },
  { id: 'graphite', name: '石墨灰', lightHex: '#8e8e93', darkHex: '#98989d', rgb: '142, 142, 147' },
];

const STORAGE_KEY_MODE = 'macos_theme_mode';
const STORAGE_KEY_ACCENT = 'macos_theme_accent';

/** 响应式主题模式状态，默认按照用户要求为 'light' 白蓝色调 */
export const currentThemeMode = ref<ThemeMode>(
  (localStorage.getItem(STORAGE_KEY_MODE) as ThemeMode) || 'light'
);

/** 响应式主题强调色，默认按照用户要求为 'blue' */
export const currentAccent = ref<AccentColor>(
  (localStorage.getItem(STORAGE_KEY_ACCENT) as AccentColor) || 'blue'
);

/** 实际生效的是否为深色 */
export const isDarkActive = ref<boolean>(false);

/** 媒体查询：系统是否开启深色模式 */
const systemDarkMediaQuery = window.matchMedia('(prefers-color-scheme: dark)');

/** 应用主题设置至 DOM 与 CSS 变量 */
export function applyTheme() {
  const root = document.documentElement;

  // 计算当前是否为深色环境
  const dark =
    currentThemeMode.value === 'dark' ||
    (currentThemeMode.value === 'system' && systemDarkMediaQuery.matches);

  isDarkActive.value = dark;

  if (dark) {
    root.classList.add('dark');
    root.setAttribute('data-theme', 'dark');
  } else {
    root.classList.remove('dark');
    root.setAttribute('data-theme', 'light');
  }

  // 计算强调色
  const accentSpec =
    ACCENT_PALETTE.find((a) => a.id === currentAccent.value) || ACCENT_PALETTE[0];
  const accentHex = dark ? accentSpec.darkHex : accentSpec.lightHex;

  root.setAttribute('data-accent', accentSpec.id);
  root.style.setProperty('--macos-accent', accentHex);
  root.style.setProperty('--macos-accent-rgb', accentSpec.rgb);
  root.style.setProperty(
    '--macos-accent-subtle',
    dark ? `rgba(${accentSpec.rgb}, 0.18)` : `rgba(${accentSpec.rgb}, 0.10)`
  );
  root.style.setProperty('--macos-border-focus', `rgba(${accentSpec.rgb}, 0.55)`);
}

/** 切换主题模式（浅色 / 深色 / 跟随系统） */
export function setThemeMode(mode: ThemeMode) {
  currentThemeMode.value = mode;
  localStorage.setItem(STORAGE_KEY_MODE, mode);
  applyTheme();
}

/** 切换强调色（蓝色 / 紫色 / 粉色等） */
export function setAccentColor(accent: AccentColor) {
  currentAccent.value = accent;
  localStorage.setItem(STORAGE_KEY_ACCENT, accent);
  applyTheme();
}

/** 全局初始化主题监听器 */
export function initTheme() {
  applyTheme();

  // 监听操作系统深色/浅色偏好变更
  systemDarkMediaQuery.addEventListener('change', () => {
    if (currentThemeMode.value === 'system') {
      applyTheme();
    }
  });

  // 监听多窗口 LocalStorage 跨窗口同步
  window.addEventListener('storage', (e) => {
    if (e.key === STORAGE_KEY_MODE && e.newValue) {
      currentThemeMode.value = e.newValue as ThemeMode;
      applyTheme();
    } else if (e.key === STORAGE_KEY_ACCENT && e.newValue) {
      currentAccent.value = e.newValue as AccentColor;
      applyTheme();
    }
  });
}
