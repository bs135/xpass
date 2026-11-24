import {create} from 'zustand'
import {api, type UpdateInfo} from '../lib/api'

/** Last update check result (startup or manual), for the Settings badge and About. */
interface UpdateState {
    info: UpdateInfo | null
    set: (info: UpdateInfo) => void
    /** Reads the result of a check that ran before the UI listened (startup). */
    refresh: () => void
}

export const useUpdate = create<UpdateState>((set) => ({
    info: null,
    set: (info) => set({info}),
    refresh: () => {
        api.updateStatus()
            .then((info) => info && set({info}))
            .catch((e) => console.error('update_status failed', e))
    },
}))
