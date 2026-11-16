import {create} from 'zustand'
import {api} from '../lib/api'

/**
 * Which screen the app shows. Secrets never live here except the recovery key
 * while the Emergency Kit is on screen; it is dropped as soon as the kit is done.
 */
export type Screen = 'loading' | 'setup' | 'kit' | 'unlock' | 'recovery' | 'newPassword' | 'unlocked'

interface SessionState {
    screen: Screen
    /** Recovery key shown by the Emergency Kit screen, null otherwise. */
    recoveryKey: string | null
    /** Load the initial screen from the vault status. */
    init: () => Promise<void>
    go: (screen: Screen) => void
    showKit: (recoveryKey: string) => void
    finishKit: () => void
    /** Called on `vault://locked` (or after our own lock call): wipe UI state. */
    onLocked: () => void
    lock: () => Promise<void>
}

export const useSession = create<SessionState>((set) => ({
    screen: 'loading',
    recoveryKey: null,
    init: async () => {
        const status = await api.vaultStatus()
        set({screen: status === 'noVault' ? 'setup' : status === 'locked' ? 'unlock' : 'unlocked'})
    },
    go: (screen) => set({screen}),
    showKit: (recoveryKey) => set({screen: 'kit', recoveryKey}),
    finishKit: () => set({screen: 'unlocked', recoveryKey: null}),
    onLocked: () => set({screen: 'unlock', recoveryKey: null}),
    lock: async () => {
        await api.lock()
        set({screen: 'unlock', recoveryKey: null})
    },
}))
