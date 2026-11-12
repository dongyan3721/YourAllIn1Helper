export interface VaultStatus {
  initialized: boolean;
  locked: boolean;
  unlocking: boolean;
  epoch: number;
  auto_lock_seconds: number;
  biometric_available: boolean;
  biometric_enrolled: boolean;
  device_key_kind: 'none' | 'macos_biometry_current_set' | 'windows_hello_gesture';
  secret_copy_available: boolean;
  capability_error: string | null;
}

export interface VaultEntrySummary {
  id: string;
  record_version: number;
  created_at: number;
  updated_at: number;
  archived: boolean;
  title: string;
  url: string;
  category: string;
  username: string;
  has_totp: boolean;
}

export interface VaultOtpCode {
  kind: 'totp' | 'hotp';
  code: string;
  digits: number;
  /** TOTP only. */
  period_secs: number | null;
  /** TOTP only: milliseconds until this code expires. */
  remaining_ms: number | null;
  /** HOTP only: the counter this code was generated with. */
  counter: number | null;
}

export interface VaultHotpAdvance {
  code: VaultOtpCode;
  record_version: number;
  /** The rewritten secret field; the open editor must adopt it to avoid saving a stale counter. */
  totp_secret: string;
}

export interface VaultEntryDraft {
  id: string;
  record_version: number;
  archived: boolean;
  title: string;
  url: string;
  category: string;
  username: string;
  password: string;
  totp_secret: string;
  notes: string;
}

export interface VaultEntryPage {
  entries: VaultEntrySummary[];
  next_cursor: string | null;
}

export interface VaultMutationResult {
  id: string;
  record_version: number;
}

export const VAULT_REVEAL_MS = 15_000;


/** Group a code for readability: 6 digits as 3+3, 8 digits as 4+4. */
export function formatOtpCode(code: string): string {
  if (code.length === 6) return `${code.slice(0, 3)} ${code.slice(3)}`;
  if (code.length === 8) return `${code.slice(0, 4)} ${code.slice(4)}`;
  return code;
}

export function emptyVaultEntry(): VaultEntryDraft {
  return {
    id: '',
    record_version: 0,
    archived: false,
    title: '',
    url: '',
    category: '',
    username: '',
    password: '',
    totp_secret: '',
    notes: '',
  };
}
