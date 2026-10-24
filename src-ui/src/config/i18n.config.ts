/**
 * 前端国际化 (i18n) 静态配置文件
 * 严格遵循「配置文件、业务逻辑、单元测试分离」原则：
 * 本文件仅声明支持的语言枚举、元数据选项、存储键名与语言代码归一化规则。
 */

/** 支持的标准语言标识类型 */
export type SupportedLocale = 'zh-CN' | 'en-US' | 'ja-JP';

/** 语言选项元数据结构 */
export interface LocaleOption {
  /** 标准 BCP 47 语言代码（与后端 config.general.language 一致） */
  code: SupportedLocale;
  /** 界面切换器简短标识 */
  shortLabel: string;
  /** 母语显示名称 */
  nativeName: string;
  /** HTML lang 属性值 */
  htmlLang: string;
}

/** 默认回退语言 */
export const DEFAULT_LOCALE: SupportedLocale = 'zh-CN';

/** 多窗口本地缓存同步键名 */
export const LOCALE_STORAGE_KEY = 'aio_locale_preference';

/** 支持的语言配置列表 */
export const LOCALE_OPTIONS: ReadonlyArray<LocaleOption> = [
  {
    code: 'zh-CN',
    shortLabel: '中',
    nativeName: '简体中文',
    htmlLang: 'zh-CN',
  },
  {
    code: 'en-US',
    shortLabel: 'EN',
    nativeName: 'English',
    htmlLang: 'en-US',
  },
  {
    code: 'ja-JP',
    shortLabel: '日',
    nativeName: '日本語',
    htmlLang: 'ja-JP',
  },
];

/**
 * 将任意外部输入的语言字符串（如系统 navigator.language 或配置文件中的简写）
 * 归一化为受支持的 SupportedLocale。
 */
export function normalizeLocale(rawLocale?: string | null): SupportedLocale {
  if (!rawLocale) {
    return DEFAULT_LOCALE;
  }
  const cleaned = rawLocale.trim().toLowerCase();
  if (cleaned.startsWith('en')) {
    return 'en-US';
  }
  if (cleaned.startsWith('ja') || cleaned.startsWith('jp')) {
    return 'ja-JP';
  }
  if (cleaned.startsWith('zh')) {
    return 'zh-CN';
  }
  return DEFAULT_LOCALE;
}
