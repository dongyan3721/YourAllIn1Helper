export interface VaultStatus {
  initialized: boolean;
  locked: boolean;
  unlocking: boolean;
  epoch: number;
  auto_lock_seconds: number;
  biometric_available: false;
  secret_copy_available: false;
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
