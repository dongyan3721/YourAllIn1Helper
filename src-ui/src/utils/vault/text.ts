import { t } from '../i18n';

/** 密码本文案只来自共享三语字典的 vault.* 命名空间。 */
export function vaultText(key: string): string {
  return t(`vault.${key}`);
}

/**
 * Rust VaultError 的用户文案映射。
 * 后端继续返回稳定的英文诊断，前端只展示当前语言的文案，避免泄露内部数据库细节。
 */
const VAULT_ERROR_PREFIXES: ReadonlyArray<readonly [string, string]> = [
  ['vault is not initialized', 'uninitialized'],
  ['vault is already initialized', 'alreadyInitialized'],
  ['vault is locked', 'locked'],
  ['another unlock is already in progress', 'unlockInProgress'],
  ['master password unlock failed', 'unlockFailed'],
  ['biometric authentication was canceled', 'biometricCanceled'],
  ['biometric authentication failed', 'biometricFailed'],
  ['biometric authentication is unavailable', 'biometricUnavailable'],
  ['biometric keychain entitlement is missing', 'biometricKeychainDenied'],
  ['biometric hardware is not enrolled', 'biometricNotEnrolled'],
  ['biometric authentication is locked out', 'biometricLockout'],
  ['biometric device key is no longer valid', 'biometricKeyInvalid'],
  ['invalid vault input: master password must contain 12 to 256 characters and not be whitespace-only', 'invalidMasterPassword'],
  ['invalid vault input: master password confirmation differs', 'confirmationMismatch'],
  ['invalid vault input: already unlocked', 'alreadyUnlocked'],
  ['invalid vault input: record version overflow', 'versionOverflow'],
  ['invalid vault input: invalid page limit', 'invalidPageLimit'],
  ['invalid vault input: invalid cursor', 'invalidCursor'],
  ['invalid vault input: search is too long', 'searchTooLong'],
  ['invalid vault input: invalid random range', 'invalidRandomRange'],
  ['invalid vault input: password length must be 8 to 256', 'invalidPasswordLength'],
  ['invalid vault input: invalid password character policy', 'invalidPasswordPolicy'],
  ['invalid vault input: invalid entry id', 'invalidEntryId'],
  ['invalid vault input: non-canonical entry id', 'invalidEntryId'],
  ['invalid vault input: title is required', 'titleRequired'],
  ['invalid vault input: field exceeds 64 KiB', 'fieldTooLarge'],
  ['invalid vault input: entry size overflow', 'entryTooLarge'],
  ['invalid vault input: entry exceeds 256 KiB', 'entryTooLarge'],
  ['invalid vault input:', 'invalidInput'],
  ['entry was not found', 'notFound'],
  ['entry version conflict', 'versionConflict'],
  ['operating system random source failed', 'randomSourceFailed'],
  ['defensive nonce reuse check rejected the operation', 'nonceReuse'],
  ['vault data integrity check failed', 'dataIntegrity'],
  ['unsupported or unsafe vault format', 'unsupportedFormat'],
  ['module is disabled', 'disabled'],
  ['feature is disabled in M1:', 'featureDisabled'],
  ['vault background task failed', 'taskJoin'],
  ['database operation failed:', 'database'],
  ['core operation failed:', 'core'],
];

export function vaultErrorKey(reason: unknown): string {
  const message = reason instanceof Error ? reason.message : String(reason);
  const normalized = message.trim();
  const match = VAULT_ERROR_PREFIXES.find(([prefix]) => normalized.startsWith(prefix));
  return `vault.errors.${match?.[1] ?? 'unknown'}`;
}

export function vaultErrorText(reason: unknown): string {
  return t(vaultErrorKey(reason));
}
