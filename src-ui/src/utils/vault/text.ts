import { t } from '../i18n';

/** 密码本文案只来自共享三语字典的 vault.* 命名空间。 */
export function vaultText(key: string): string {
  return t(`vault.${key}`);
}
