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

/** Error shape returned by every command (`src-tauri/src/error.rs`). */
export type AppError =
    | {code: 'no_vault'}
    | {code: 'vault_exists'}
    | {code: 'locked'}
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
}
