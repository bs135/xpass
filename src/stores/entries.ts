import {create} from 'zustand'
import {api, type EntryDetail, type EntryInput, type EntrySummary} from '../lib/api'

/**
 * Entry list and selection. Holds metadata and details only: passwords never enter
 * this store (the edit form keeps a typed password in its own local state).
 * `reset()` runs on lock.
 */
export type Mode = 'view' | 'edit' | 'new'

interface EntriesState {
    entries: EntrySummary[]
    selectedId: string | null
    detail: EntryDetail | null
    mode: Mode
    query: string
    favoritesOnly: boolean
    load: () => Promise<void>
    /** After a sync brought remote changes: reload the list and the shown entry (not while editing). */
    refresh: () => Promise<void>
    select: (id: string | null) => Promise<void>
    startNew: () => void
    startEdit: () => void
    cancelEdit: () => void
    /** Saves the form, reloads the list and shows the saved entry. */
    save: (input: EntryInput) => Promise<void>
    remove: (id: string) => Promise<void>
    toggleFavorite: (id: string, favorite: boolean) => Promise<void>
    setQuery: (query: string) => void
    setFavoritesOnly: (value: boolean) => void
    reset: () => void
}

const initial = {
    entries: [] as EntrySummary[],
    selectedId: null as string | null,
    detail: null as EntryDetail | null,
    mode: 'view' as Mode,
    query: '',
    favoritesOnly: false,
}

export const useEntries = create<EntriesState>((set, get) => ({
    ...initial,
    load: async () => {
        set({entries: await api.listEntries()})
    },
    refresh: async () => {
        await get().load()
        const {selectedId, mode, entries} = get()
        if (selectedId === null || mode !== 'view') return
        if (entries.some((e) => e.id === selectedId)) await get().select(selectedId)
        else set({selectedId: null, detail: null})
    },
    select: async (id) => {
        if (id === null) {
            set({selectedId: null, detail: null, mode: 'view'})
            return
        }
        set({selectedId: id, mode: 'view'})
        const detail = await api.getEntry(id)
        // Ignore a slow response for an entry that is no longer selected.
        if (get().selectedId === id) set({detail})
    },
    startNew: () => set({mode: 'new'}),
    startEdit: () => {
        if (get().detail) set({mode: 'edit'})
    },
    cancelEdit: () => set({mode: 'view'}),
    save: async (input) => {
        const id = await api.saveEntry(input)
        await get().load()
        await get().select(id)
    },
    remove: async (id) => {
        await api.deleteEntry(id)
        await get().load()
        if (get().selectedId === id) set({selectedId: null, detail: null, mode: 'view'})
    },
    toggleFavorite: async (id, favorite) => {
        await api.setFavorite(id, favorite)
        await get().load()
        if (get().selectedId === id) await get().select(id)
    },
    setQuery: (query) => set({query}),
    setFavoritesOnly: (favoritesOnly) => set({favoritesOnly}),
    reset: () => set(initial),
}))
