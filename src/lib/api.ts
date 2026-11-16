/**
 * Typed wrappers around Tauri commands. All IPC goes through this module so the
 * contract with `src-tauri/src/commands` stays in one place.
 */
import {invoke} from '@tauri-apps/api/core'

export interface AppInfo {
    version: string
    formatVersion: number
}

export type VaultStatus = 'noVault' | 'locked' | 'unlocked'

export interface RecoveryKit {
    /** Printable recovery key. Show it once; never store it. */
    recoveryKey: string
}

export interface Strength {
    /** 0 (very weak) to 4 (very strong); setup requires at least 3. */
    score: number
    /** English feedback from zxcvbn. */
    warning: string | null
    suggestions: string[]
}

export const MIN_STRENGTH = 3

/** Row in the entry list: metadata only. */
export interface EntrySummary {
    id: string
    title: string
    username: string
    urlHost: string | null
    tags: string[]
    favorite: boolean
    updatedAt: number
}

/** Entry details without the password or secret custom field values. */
export interface EntryDetail {
    id: string
    title: string
    username: string
    hasPassword: boolean
    urls: string[]
    notes: string
    tags: string[]
    favorite: boolean
    fields: {name: string; value: string | null; secret: boolean}[]
    historyCount: number
    createdAt: number
    updatedAt: number
}

export interface EntryInput {
    /** null creates a new entry. */
    id: string | null
    title: string
    username: string
    /** null keeps the current password. */
    password: string | null
    urls: string[]
    notes: string
    tags: string[]
    favorite: boolean
}

/** Part of an entry that can be copied (all) or revealed (password, secret field). */
export type FieldRef =
    | {kind: 'username'}
    | {kind: 'password'}
    | {kind: 'url'; index: number}
    | {kind: 'field'; index: number}

export type GeneratorOptions =
    | {
          mode: 'chars'
          length: number
          lower: boolean
          upper: boolean
          digits: boolean
          symbols: boolean
          excludeAmbiguous: boolean
      }
    | {mode: 'passphrase'; words: number; separator: string; capitalize: boolean; addDigit: boolean}

export interface Generated {
    password: string
    entropyBits: number
}

/** Seconds before a copied secret is cleared from the clipboard. */
export const CLIPBOARD_CLEAR_SECONDS = 20

/** Error shape returned by every command (`src-tauri/src/error.rs`). */
export type AppError =
    | {code: 'no_vault'}
    | {code: 'vault_exists'}
    | {code: 'locked'}
    | {code: 'not_found'}
    | {code: 'open_failed'}
    | {code: 'clipboard'}
    | {code: 'weak_password'}
    | {code: 'invalid_input'; field: string}
    | {code: 'bad_credentials'}
    | {code: 'bad_recovery_key'}
    | {code: 'throttled'; retryAfterMs: number}
    | {code: 'tampered'}
    | {code: 'invalid_file'}
    | {code: 'unsupported_version'; version: number}
    | {code: 'no_such_slot'}
    | {code: 'different_vault'}
    | {code: 'key_changed'}
    | {code: 'io'; kind: string}
    | {code: 'internal'}

export function isAppError(e: unknown): e is AppError {
    return typeof e === 'object' && e !== null && typeof (e as {code?: unknown}).code === 'string'
}

/** Emitted when the vault locks; clear all entry state when received. */
export const EVENT_LOCKED = 'vault://locked'

export const api = {
    appInfo: () => invoke<AppInfo>('app_info'),
    vaultStatus: () => invoke<VaultStatus>('vault_status'),
    createVault: (password: string) => invoke<RecoveryKit>('create_vault', {password}),
    unlockPassword: (password: string) => invoke<void>('unlock_password', {password}),
    unlockRecovery: (recoveryKey: string) => invoke<void>('unlock_recovery', {recoveryKey}),
    /** `currentPassword` may be null right after a recovery-key unlock. */
    changePassword: (currentPassword: string | null, newPassword: string) =>
        invoke<void>('change_password', {currentPassword, newPassword}),
    /** New Emergency Kit. `currentPassword` may be null right after a recovery-key unlock. */
    newRecoveryKey: (currentPassword: string | null) => invoke<RecoveryKit>('new_recovery_key', {currentPassword}),
    passwordStrength: (password: string) => invoke<Strength>('password_strength', {password}),
    lock: () => invoke<void>('lock'),
    listEntries: () => invoke<EntrySummary[]>('list_entries'),
    getEntry: (id: string) => invoke<EntryDetail>('get_entry', {id}),
    /** Returns the entry id. */
    saveEntry: (input: EntryInput) => invoke<string>('save_entry', {input}),
    deleteEntry: (id: string) => invoke<void>('delete_entry', {id}),
    setFavorite: (id: string, favorite: boolean) => invoke<void>('set_favorite', {id, favorite}),
    openUrl: (id: string, index: number) => invoke<void>('open_url', {id, index}),
    copyField: (id: string, field: FieldRef) => invoke<void>('copy_field', {id, field}),
    revealSecret: (id: string, field: FieldRef) =>
        invoke<{value: string}>('reveal_secret', {id, field}).then((s) => s.value),
    generatePassword: (options: GeneratorOptions) => invoke<Generated>('generate_password', {options}),
    copyText: (text: string) => invoke<void>('copy_text', {text}),
}
