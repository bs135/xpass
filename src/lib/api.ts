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

/** Rotation result: new Emergency Kit, and whether Windows Hello was turned off. */
export interface RotatedKit extends RecoveryKit {
    helloDropped: boolean
}

/** Windows Hello state of the vault (readable while locked). */
export interface HelloInfo {
    /** The vault has a Hello key slot. */
    enrolled: boolean
    /** A Hello unlock found the credential missing or changed; use the password. */
    broken: boolean
    /** Hello was turned off (VaultKey rotated or Hello reset): offer to set it up again. */
    reenrollSuggested: boolean
}

export interface HelloStatus extends HelloInfo {
    /** Windows Hello is set up and usable on this PC. */
    supported: boolean
}

export interface SessionInfo {
    status: VaultStatus
    /** Set only while locked. */
    lockReason: LockReason | null
    hello: HelloInfo
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

/** App settings (`settings.json`, no secrets). */
export interface Settings {
    theme: 'system' | 'light' | 'dark'
    /** null follows the OS language. */
    language: 'en' | 'vi' | null
    /** null = never. */
    autoLockMinutes: number | null
    clipboardClearSeconds: number
    /** The close button hides xpass to the tray instead of quitting. */
    closeToTray: boolean
    /** Lock when minimized or hidden to the tray. */
    lockOnMinimize: boolean
    /** Set by the app when an Emergency Kit is generated (Unix ms). */
    lastKitAt: number | null
}

export const AUTO_LOCK_CHOICES = [1, 5, 15, 30, 60] as const
export const CLIPBOARD_CHOICES = [10, 20, 30, 60, 90] as const

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
    | {code: 'hello_unavailable'}
    | {code: 'hello_not_found'}
    | {code: 'hello_canceled'}
    | {code: 'hello_device_locked'}
    | {code: 'hello_busy'}
    | {code: 'hello_failed'}
    | {code: 'io'; kind: string}
    | {code: 'internal'}

export function isAppError(e: unknown): e is AppError {
    return typeof e === 'object' && e !== null && typeof (e as {code?: unknown}).code === 'string'
}

/** Emitted when the vault locks; clear all entry state when received. */
export const EVENT_LOCKED = 'vault://locked'

/** Why the vault locked (payload of `vault://locked`). */
export type LockReason = 'manual' | 'idle' | 'sessionLocked' | 'displayOff' | 'suspend' | 'minimized' | 'exit'

export const api = {
    appInfo: () => invoke<AppInfo>('app_info'),
    openProjectPage: () => invoke<void>('open_project_page'),
    /** Status, why the vault was locked (while locked) and the Windows Hello state. */
    vaultStatus: () => invoke<SessionInfo>('vault_status'),
    createVault: (password: string) => invoke<RecoveryKit>('create_vault', {password}),
    unlockPassword: (password: string) => invoke<void>('unlock_password', {password}),
    unlockRecovery: (recoveryKey: string) => invoke<void>('unlock_recovery', {recoveryKey}),
    /** `currentPassword` may be null right after a recovery-key unlock. */
    changePassword: (currentPassword: string | null, newPassword: string) =>
        invoke<void>('change_password', {currentPassword, newPassword}),
    /** New Emergency Kit. `currentPassword` may be null right after a recovery-key unlock. */
    newRecoveryKey: (currentPassword: string | null) => invoke<RecoveryKit>('new_recovery_key', {currentPassword}),
    /** Compromise response: new VaultKey and recovery key; the password stays the same. Turns Hello off. */
    rotateVaultKey: (currentPassword: string) => invoke<RotatedKit>('rotate_vault_key', {currentPassword}),
    /** Hello state plus whether Hello is usable on this PC (asks Windows; may take a moment). */
    helloStatus: () => invoke<HelloStatus>('hello_status'),
    /** Shows the Windows Hello dialog; resolves once the vault is unlocked. */
    unlockHello: () => invoke<void>('unlock_hello'),
    /** Turns Hello on (master password check, then the Windows Hello dialog). */
    helloEnable: (currentPassword: string | null) => invoke<void>('hello_enable', {currentPassword}),
    helloDisable: () => invoke<void>('hello_disable'),
    helloDismissReenroll: () => invoke<void>('hello_dismiss_reenroll'),
    getSettings: () => invoke<Settings>('get_settings'),
    updateSettings: (settings: Settings) => invoke<Settings>('update_settings', {settings}),
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
