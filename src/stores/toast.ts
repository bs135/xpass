import {create} from 'zustand'

/** One short status message at a time (e.g. "Copied, clears in 20 s"). Never secrets. */
interface ToastState {
    message: string | null
    show: (message: string, ms?: number) => void
}

let timer: number | undefined

export const useToast = create<ToastState>((set) => ({
    message: null,
    show: (message, ms = 2500) => {
        window.clearTimeout(timer)
        set({message})
        timer = window.setTimeout(() => set({message: null}), ms)
    },
}))
