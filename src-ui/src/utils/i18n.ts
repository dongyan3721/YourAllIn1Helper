/**
 * 前端国际化 (i18n) 运行时引擎与多窗口同步服务
 * 职责：
 * 1. 加载各语言的独立 JSON 字典配置文件（zh-CN / en-US / ja-JP）；
 * 2. 提供响应式语言状态 `currentLocale` 与带参数插值的纯函数/响应式翻译器 `t(key, params)`；
 * 3. 通过 LocalStorage 的 `storage` 事件实现主控台、剪贴板弹窗、独立置顶便笺等多窗口秒级同步；
 * 4. 通过 `kernel_update_general_config` IPC 将用户选择的语言持久化至后端 `config.json`。
 */

import { ref } from 'vue';
import {
  DEFAULT_LOCALE,
  LOCALE_OPTIONS,
  LOCALE_STORAGE_KEY,
  normalizeLocale,
  type SupportedLocale,
} from '../config/i18n.config';
import zhCN from '../locales/zh-CN.json';
import enUS from '../locales/en-US.json';
import jaJP from '../locales/ja-JP.json';
import { invokeIpc } from '../api/ipc';

/** 语言字典映射表（只读） */
export const LOCALE_MESSAGES: Record<SupportedLocale, Record<string, any>> = {
  'zh-CN': zhCN,
  'en-US': enUS,
  'ja-JP': jaJP,
};

/** 读取初始语言（优先读取本地缓存，其次跟随系统语言，兜底默认 zh-CN） */
function resolveInitialLocale(): SupportedLocale {
  if (typeof window !== 'undefined' && window.localStorage) {
    const cached = window.localStorage.getItem(LOCALE_STORAGE_KEY);
    if (cached) {
      return normalizeLocale(cached);
    }
  }
  return DEFAULT_LOCALE;
}

/** 全局响应式当前语言标识 */
export const currentLocale = ref<SupportedLocale>(resolveInitialLocale());

/**
 * 根据点分路径（如 "overview.metrics.memoryLabel"）从嵌套字典对象中解析文本
 */
export function resolveNestedKey(dict: Record<string, any>, keyPath: string): string | undefined {
  const segments = keyPath.split('.');
  let current: any = dict;
  for (const seg of segments) {
    if (current == null || typeof current !== 'object' || !(seg in current)) {
      return undefined;
    }
    current = current[seg];
  }
  return typeof current === 'string' ? current : undefined;
}

/**
 * 格式化命名占位符（如 "{count} 个原生窗口" -> "2 个原生窗口"）
 */
export function interpolateTemplate(
  template: string,
  params?: Record<string, string | number>
): string {
  if (!params) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (_, token: string) => {
    const val = params[token];
    return val !== undefined && val !== null ? String(val) : `{${token}}`;
  });
}

/**
 * 纯函数翻译解析（可供单元测试与非 DOM 环境直接调用）
 */
export function translateWithLocale(
  locale: SupportedLocale,
  key: string,
  params?: Record<string, string | number>
): string {
  const primaryDict = LOCALE_MESSAGES[locale] || LOCALE_MESSAGES[DEFAULT_LOCALE];
  const fallbackDict = LOCALE_MESSAGES[DEFAULT_LOCALE];

  const rawTemplate =
    resolveNestedKey(primaryDict, key) ??
    resolveNestedKey(fallbackDict, key) ??
    key;

  return interpolateTemplate(rawTemplate, params);
}

/**
 * 响应式翻译函数（在 Vue 模板与计算属性中调用会自动追踪 `currentLocale.value` 变化）
 */
export function t(key: string, params?: Record<string, string | number>): string {
  return translateWithLocale(currentLocale.value, key, params);
}

/**
 * 将当前语言同步写入 DOM `<html lang="...">` 与文档标题
 */
export function applyLocaleToDocument(locale: SupportedLocale = currentLocale.value) {
  if (typeof document === 'undefined') {
    return;
  }
  const option = LOCALE_OPTIONS.find((item) => item.code === locale) || LOCALE_OPTIONS[0];
  document.documentElement.setAttribute('lang', option.htmlLang);
}

/**
 * 切换当前语言，同步多窗口缓存并可选持久化至 Rust 微内核 `config.json`
 */
export async function setLocale(
  nextLocale: string,
  persistToBackend: boolean = true
): Promise<SupportedLocale> {
  const normalized = normalizeLocale(nextLocale);
  currentLocale.value = normalized;

  if (typeof window !== 'undefined' && window.localStorage) {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, normalized);
  }
  applyLocaleToDocument(normalized);

  if (persistToBackend) {
    try {
      await invokeIpc('kernel_update_general_config', { language: normalized });
    } catch (err) {
      console.error('同步语言配置至后端 config.json 失败:', err);
    }
  }

  return normalized;
}

/**
 * 当从后端 `kernel_get_status` 获取到 `config.general.language` 时，同步更新前端语言状态（不重复触发写回）
 */
export function syncLocaleFromBackendConfig(backendLanguage?: string | null) {
  if (!backendLanguage) {
    return;
  }
  const normalized = normalizeLocale(backendLanguage);
  if (currentLocale.value !== normalized) {
    currentLocale.value = normalized;
    if (typeof window !== 'undefined' && window.localStorage) {
      window.localStorage.setItem(LOCALE_STORAGE_KEY, normalized);
    }
    applyLocaleToDocument(normalized);
  }
}

let isInitialized = false;

/**
 * 初始化全局多语言监听器（跨 Tauri WebView 窗口实时同步语言切换）
 */
export function initI18n() {
  applyLocaleToDocument(currentLocale.value);

  if (isInitialized || typeof window === 'undefined') {
    return;
  }
  isInitialized = true;

  window.addEventListener('storage', (event: StorageEvent) => {
    if (event.key === LOCALE_STORAGE_KEY && event.newValue) {
      const updated = normalizeLocale(event.newValue);
      currentLocale.value = updated;
      applyLocaleToDocument(updated);
    }
  });
}

export { LOCALE_OPTIONS, DEFAULT_LOCALE, normalizeLocale, type SupportedLocale };
