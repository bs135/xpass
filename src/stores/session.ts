import {create} from 'zustand'
import {api, type LockReason} from '../lib/api'
import {useEntries} from './entries'

/**
 * Which screen the app shows. Secrets never live here except the recovery key
 * while the Emergency Kit is on screen; it is dropped as soon as the kit is done.
 */
export type Screen = 'loading' | 'setup' | 'kit' | 'unlock' | 'recovery' | 'newPassword' | 'unlocked' | 'settings'

interface SessionState {
    screen: Screen
    /** Recovery key shown by the Emergency Kit screen, null otherwise. */
    recoveryKey: string | null
    /** Why the vault was locked last (shown on the unlock screen); null after unlock. */
    lockReason: LockReason | null
    /** Load the initial screen from the vault status. */
    init: () => Promise<void>
    /**
     * Re-read the status (window shown or focused again). Catches a lock whose
     * `vault://locked` event was lost, e.g. while Windows was asleep.
     */
    sync: () => Promise<void>
    go: (screen: Screen) => void
    showKit: (recoveryKey: string) => void
    finishKit: () => void
    /** Called on `vault://locked` (or after our own lock call): wipe UI state. */
    onLocked: (reason?: LockReason) => void
    lock: () => Promise<void>
}

export const useSession = create<SessionState>((set, get) => ({
    screen: 'loading',
    recoveryKey: null,
    lockReason: null,
    init: async () => {
        const {status, lockReason} = await api.vaultStatus()
        set({screen: status === 'noVault' ? 'setup' : status === 'locked' ? 'unlock' : 'unlocked', lockReason})
    },
    sync: async () => {
        const {status, lockReason} = await api.vaultStatus()
        if (status !== 'locked') return
        const {screen, onLocked} = get()
        if (screen === 'unlock' || screen === 'recovery') {
            if (lockReason) set({lockReason})
        } else {
            onLocked(lockReason ?? undefined)
        }
    },
    go: (screen) => set(screen === 'unlocked' ? {screen, lockReason: null} : {screen}),
    showKit: (recoveryKey) => set({screen: 'kit', recoveryKey}),
    finishKit: () => set({screen: 'unlocked', recoveryKey: null, lockReason: null}),
    onLocked: (reason) => {
        useEntries.getState().reset()
        set({screen: 'unlock', recoveryKey: null, lockReason: reason ?? 'manual'})
    },
    lock: async () => {
        await api.lock()
        useEntries.getState().reset()
        set({screen: 'unlock', recoveryKey: null, lockReason: 'manual'})
    },
}))
