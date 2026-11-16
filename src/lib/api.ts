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

/** Error shape returned by every command (`src-tauri/src/error.rs`). */
export type AppError =
    | {code: 'no_vault'}
    | {code: 'vault_exists'}
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
    /** Returns the recovery key for the Emergency Kit. Show it once; never store it. */
    createVault: (password: string) => invoke<{recoveryKey: string}>('create_vault', {password}),
    unlockPassword: (password: string) => invoke<void>('unlock_password', {password}),
    unlockRecovery: (recoveryKey: string) => invoke<void>('unlock_recovery', {recoveryKey}),
    lock: () => invoke<void>('lock'),
}
