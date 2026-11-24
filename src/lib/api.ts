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
    /** Old password at `index` (oldest first); `at` must match or the call fails with not_found. */
    | {kind: 'history'; index: number; at: number}

/** Password history item without the password (`list_history`, newest first). */
export interface HistoryItemView {
    index: number
    /** Unix ms when this password was replaced. */
    at: number
    /** Only when it differs from the current username. */
    username: string | null
    note: 'conflict' | 'restore' | 'import' | null
    hasPassword: boolean
}

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
    /** Ask GitHub for a newer release at startup, at most once a day (off by default). */
    checkUpdatesOnStartup: boolean
    /** Set by the app when an update check ran (Unix ms). */
    lastUpdateCheckAt: number | null
}

/** Result of an update check (Rust asks the GitHub Releases API; nothing is downloaded). */
export interface UpdateInfo {
    current: string
    latest: string
    newer: boolean
    url: string
    checkedAt: number
}

/** Sync setup (`settings.json` + whether a token is stored). The token itself never comes back. */
export interface SyncConfig {
    /** `owner/name`; empty when sync is not set up. */
    repo: string
    /** Empty = the repository's default branch. */
    branch: string
    path: string
    enabled: boolean
    /** A GitHub token is stored in Windows Credential Manager. */
    hasToken: boolean
}

/** What "Test connection" found at the vault path. */
export type RemoteFile = 'missing' | 'sameVault' | 'otherVault' | 'found' | 'notAVault'

export interface ConnectionReport {
    /** false for a public repository. */
    private: boolean
    /** Branch that will be used (configured or the repository default). */
    branch: string
    branchExists: boolean
    remote: RemoteFile
}

/** Sync state shown in the header, the tray and Settings (`sync://status`). */
export type SyncPhase =
    | 'off'
    | 'idle'
    | 'syncing'
    | 'offline'
    | 'noToken'
    | 'tokenInvalid'
    | 'needsPassword'
    | 'differentVault'
    | 'error'

export interface SyncStatus {
    phase: SyncPhase
    /** Local changes GitHub has not seen yet. */
    dirty: boolean
    /** Unix ms of the last successful sync. */
    lastSyncAt: number | null
    error: AppError | null
    /** The master password was changed on another device and adopted here. */
    passwordChanged: boolean
}

/** Whether a local backup can be restored (`list_backups`). */
export type BackupKind = 'current' | 'olderKey' | 'otherVault' | 'unreadable'

export interface BackupInfo {
    /** File name in the backups folder; the id for `restoreBackup`. */
    name: string
    /** Unix ms when this version was saved. */
    savedAt: number | null
    size: number
    kind: BackupKind
    /** Entries in that version, when it opens with the current key. */
    entries: number | null
}

/** CSV import presets (`xpass-interop`, plan I4). */
export type ImportPreset = 'bitwarden' | 'chrome' | 'keepassxc' | 'xpass' | 'generic'
export const IMPORT_PRESETS: ImportPreset[] = ['bitwarden', 'chrome', 'keepassxc', 'xpass', 'generic']

/** Generic preset: column index (0-based) for each entry field, or null. */
export interface ImportMapping {
    title: number | null
    username: number | null
    password: number | null
    url: number | null
    notes: number | null
    tags: number | null
    favorite: number | null
    /** What happens to the columns not mapped above. */
    unmapped: 'ignore' | 'notes' | 'fields'
}

export type ImportRowWarning = 'titleFromUrl' | 'titleFromUsername' | 'invalidDate' | 'invalidFields'
export type ImportDuplicate = 'none' | 'exact' | 'possible'

/** One row of the import preview: no password, notes or custom-field values. */
export interface ImportPreviewRow {
    index: number
    line: number
    title: string
    username: string
    urlHost: string | null
    hasPassword: boolean
    duplicate: ImportDuplicate
    warnings: ImportRowWarning[]
}

export interface ImportPreview {
    /** File name only, no folder. */
    fileName: string
    detected: ImportPreset
    preset: ImportPreset
    /** Column names; generic preset only (empty otherwise). */
    headers: string[]
    /** Set for the generic preset only. */
    mapping: ImportMapping | null
    rows: ImportPreviewRow[]
    skipped: {line: number; reason: 'empty' | 'unsupportedType'}[]
    counts: {new: number; exact: number; possible: number}
}

/** What to do with possible duplicates (same site and username, other password). */
export type DuplicateChoice = 'skip' | 'new' | 'update'

export interface ImportSummary {
    added: number
    updated: number
    skipped: number
}

export type ExportFormat = 'csv' | 'kdbx'

/** How the user confirms an export (checked in Rust before the save dialog). */
export type ExportAuth = {method: 'password'; password: string} | {method: 'hello'}

export interface ExportResult {
    /** File name only, no folder. */
    fileName: string
    count: number
}

export interface SyncTarget {
    repo: string
    branch: string
    path: string
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
    | {code: 'secret_store'}
    | {code: 'sync_not_configured'}
    | {code: 'sync_offline'}
    | {code: 'sync_unauthorized'}
    | {code: 'sync_forbidden'}
    | {code: 'sync_rate_limited'; retryAfterS: number | null}
    | {code: 'sync_repo_not_found'}
    | {code: 'sync_failed'}
    | {code: 'sync_remote_missing'}
    | {code: 'sync_required'}
    | {code: 'import_too_large'; maxMb: number; maxRows: number}
    | {code: 'import_invalid_csv'; line: number | null}
    | {code: 'import_encoding'}
    | {code: 'import_empty'}
    | {code: 'import_expired'}
    | {code: 'dialog_failed'}
    | {code: 'io'; kind: string}
    | {code: 'internal'}

export function isAppError(e: unknown): e is AppError {
    return typeof e === 'object' && e !== null && typeof (e as {code?: unknown}).code === 'string'
}

/** Sync status changed (payload: SyncStatus). */
export const EVENT_SYNC_STATUS = 'sync://status'
/** Result of the optional startup update check ({@link UpdateInfo}). */
export const EVENT_UPDATE_STATUS = 'update://status'
/** A sync brought changes from another device: reload the entry list. */
export const EVENT_VAULT_CHANGED = 'vault://changed'

/** Emitted when the vault locks; clear all entry state when received. */
export const EVENT_LOCKED = 'vault://locked'

/** Why the vault locked (payload of `vault://locked`). */
export type LockReason = 'manual' | 'idle' | 'sessionLocked' | 'displayOff' | 'suspend' | 'minimized' | 'exit'

export const api = {
    appInfo: () => invoke<AppInfo>('app_info'),
    openProjectPage: () => invoke<void>('open_project_page'),
    checkUpdate: () => invoke<UpdateInfo>('check_update'),
    updateStatus: () => invoke<UpdateInfo | null>('update_status'),
    openReleasePage: () => invoke<void>('open_release_page'),
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
    listHistory: (id: string) => invoke<HistoryItemView[]>('list_history', {id}),
    /** Makes an old password current; the current one moves into the history. */
    useHistoryPassword: (id: string, index: number, at: number) =>
        invoke<void>('use_history_password', {id, index, at}),
    deleteEntry: (id: string) => invoke<void>('delete_entry', {id}),
    setFavorite: (id: string, favorite: boolean) => invoke<void>('set_favorite', {id, favorite}),
    openUrl: (id: string, index: number) => invoke<void>('open_url', {id, index}),
    copyField: (id: string, field: FieldRef) => invoke<void>('copy_field', {id, field}),
    revealSecret: (id: string, field: FieldRef) =>
        invoke<{value: string}>('reveal_secret', {id, field}).then((s) => s.value),
    generatePassword: (options: GeneratorOptions) => invoke<Generated>('generate_password', {options}),
    copyText: (text: string) => invoke<void>('copy_text', {text}),
    syncGetConfig: () => invoke<SyncConfig>('sync_get_config'),
    syncConfigure: (target: SyncTarget) => invoke<SyncConfig>('sync_configure', {...target}),
    /** Stores the token in Windows Credential Manager; it is never returned. */
    syncSetToken: (token: string) => invoke<SyncConfig>('sync_set_token', {token}),
    /** Deletes the token and the sync setup; the local vault stays. */
    syncDisconnect: () => invoke<SyncConfig>('sync_disconnect'),
    /** `token` null uses the stored token. */
    syncTestConnection: (target: SyncTarget, token: string | null) =>
        invoke<ConnectionReport>('sync_test_connection', {...target, token}),
    syncOpenTokenPage: () => invoke<void>('sync_open_token_page'),
    syncStatus: () => invoke<SyncStatus>('sync_status'),
    /** Starts a sync; the result arrives as `sync://status`. */
    syncNow: () => invoke<SyncStatus>('sync_now'),
    syncDismissPasswordChanged: () => invoke<void>('sync_dismiss_password_changed'),
    /** "Use my vault from GitHub" (no vault on this PC): download, open with `password`, turn sync on. */
    syncJoin: (target: SyncTarget, token: string, password: string) =>
        invoke<void>('sync_join', {...target, token, password}),
    /** The vault key was changed on another device: `password` is the master password as set there. */
    syncResolveKeyChange: (password: string) => invoke<SyncStatus>('sync_resolve_key_change', {password}),
    /** Replaces this PC's vault with the one on GitHub (kept as a backup); `password` opens the GitHub vault. */
    syncReplaceLocal: (password: string) => invoke<void>('sync_replace_local', {password}),
    /** Local backups, newest first (needs the vault unlocked). */
    listBackups: () => invoke<BackupInfo[]>('list_backups'),
    /** Restores a backup as a new change. `password` only for a backup from before a key change. */
    restoreBackup: (name: string, password: string | null) => invoke<void>('restore_backup', {name, password}),
    /** Open dialog, then parse the file in Rust. null: the dialog was canceled. */
    importOpen: (preset: ImportPreset | null) => invoke<ImportPreview | null>('import_open', {preset}),
    /** Re-reads the pending file with another preset / generic mapping (null guesses it). */
    importSetMapping: (preset: ImportPreset, mapping: ImportMapping | null) =>
        invoke<ImportPreview>('import_set_mapping', {preset, mapping}),
    importCommit: (duplicates: DuplicateChoice, excluded: number[]) =>
        invoke<ImportSummary>('import_commit', {options: {duplicates, excluded}}),
    importCancel: () => invoke<void>('import_cancel'),
    /** Deletes the CSV file that was imported (only that file). */
    importDeleteSource: () => invoke<void>('import_delete_source'),
    /** Re-auth, then the save dialog, then the file is written in Rust. null: the dialog was canceled. */
    exportVault: (format: ExportFormat, auth: ExportAuth, kdbxPassword: string | null) =>
        invoke<ExportResult | null>('export_vault', {format, auth, kdbxPassword}),
}
