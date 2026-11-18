import {create} from 'zustand'
import {api, type SyncStatus} from '../lib/api'

const OFF: SyncStatus = {phase: 'off', dirty: false, lastSyncAt: null, error: null, passwordChanged: false}

interface SyncState {
    status: SyncStatus
    /** From the `sync://status` event. */
    set: (status: SyncStatus) => void
    /** Re-reads the status (startup, window shown again, after a setup change). */
    refresh: () => void
    /** "Sync now"; resolves when the request is accepted, the result comes as an event. */
    syncNow: () => Promise<void>
}

export const useSync = create<SyncState>((set) => ({
    status: OFF,
    set: (status) => set({status}),
    refresh: () => {
        api.syncStatus()
            .then((status) => set({status}))
            .catch((e) => console.error('sync_status failed', e))
    },
    syncNow: async () => {
        set({status: await api.syncNow()})
    },
}))
