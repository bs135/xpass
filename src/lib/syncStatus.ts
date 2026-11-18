import type {TFunction} from 'i18next'
import type {SyncStatus} from './api'
import {errorMessage} from './errors'

/** What the sync indicator shows: an i18n key under `sync.status` and the click action. */
export interface SyncView {
    key: 'syncing' | 'synced' | 'pending' | 'offline' | 'noToken' | 'tokenInvalid' | 'needsPassword' | 'differentVault' | 'error'
    /** Problems only the user can fix open Settings; the rest start a sync. */
    action: 'syncNow' | 'openSettings'
    /** Shown with a warning colour. */
    warn: boolean
}

export function syncView(status: SyncStatus): SyncView | null {
    switch (status.phase) {
        case 'off':
            return null
        case 'syncing':
            return {key: 'syncing', action: 'syncNow', warn: false}
        case 'idle':
            return {key: status.dirty ? 'pending' : 'synced', action: 'syncNow', warn: false}
        case 'offline':
            return {key: 'offline', action: 'syncNow', warn: false}
        case 'noToken':
        case 'tokenInvalid':
        case 'needsPassword':
        case 'differentVault':
            return {key: status.phase, action: 'openSettings', warn: true}
        case 'error':
            return {key: 'error', action: 'syncNow', warn: true}
    }
}

/** Minutes since the last sync, or null when unknown. */
export function minutesSince(lastSyncAt: number | null, now: number): number | null {
    if (lastSyncAt === null) return null
    return Math.max(0, Math.floor((now - lastSyncAt) / 60_000))
}

/** One-line description of the sync state, e.g. "Synced 3 min ago." */
export function syncText(t: TFunction, status: SyncStatus, now: number): string {
    const view = syncView(status)
    if (!view) return t('sync.status.off')
    const minutes = minutesSince(status.lastSyncAt, now)
    let text = t(`sync.status.${view.key}`)
    if (view.key === 'synced' && minutes !== null) text = t('sync.status.syncedAgo', {count: minutes})
    if (status.error && view.key !== 'synced') text += ` ${errorMessage(t, status.error)}`
    return text
}
