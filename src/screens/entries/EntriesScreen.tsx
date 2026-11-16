import {useEffect, useMemo, useRef, useState} from 'react'
import {KeyRound} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Alert} from '../../components/ui'
import {errorMessage} from '../../lib/errors'
import {filterEntries} from '../../lib/search'
import {useEntries} from '../../stores/entries'
import {useSession} from '../../stores/session'
import {EntryForm} from './EntryForm'
import {EntryList} from './EntryList'
import {EntryView} from './EntryView'

/** Two-pane entry manager: list + search on the left, details or form on the right. */
export function EntriesScreen() {
    const {t} = useTranslation()
    const {entries, query, favoritesOnly, selectedId, detail, mode, load, remove} = useEntries()
    const lock = useSession((s) => s.lock)
    const searchRef = useRef<HTMLInputElement>(null)
    const [confirmDelete, setConfirmDelete] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const visible = useMemo(() => filterEntries(entries, query, favoritesOnly), [entries, query, favoritesOnly])

    useEffect(() => {
        load().catch((e) => setError(errorMessage(t, e)))
    }, [load, t])

    useShortcuts({
        search: () => searchRef.current?.focus(),
        newEntry: () => useEntries.getState().startNew(),
        edit: () => useEntries.getState().startEdit(),
        remove: () => {
            const s = useEntries.getState()
            if (s.selectedId && s.mode === 'view') setConfirmDelete(true)
        },
        lock: () => void lock().catch((e) => console.error('lock failed', e)),
    })

    const doDelete = () => {
        setConfirmDelete(false)
        if (selectedId) remove(selectedId).catch((e) => setError(errorMessage(t, e)))
    }

    return (
        <div className="flex min-h-0 flex-1">
            <EntryList visible={visible} searchRef={searchRef}/>
            <section className="min-w-0 flex-1 overflow-auto">
                {error && (
                    <div className="p-4">
                        <Alert>{error}</Alert>
                    </div>
                )}
                {mode === 'new' ? (
                    <EntryForm key="new" detail={null}/>
                ) : mode === 'edit' && detail ? (
                    <EntryForm key={`edit-${detail.id}`} detail={detail}/>
                ) : detail && detail.id === selectedId ? (
                    <EntryView detail={detail} onDelete={() => setConfirmDelete(true)}/>
                ) : (
                    <EmptyState hasEntries={entries.length > 0}/>
                )}
            </section>
            {confirmDelete && detail && (
                <ConfirmDialog
                    title={t('entries.deleteTitle')}
                    message={t('entries.deleteMessage', {title: detail.title})}
                    confirmLabel={t('actions.delete')}
                    onConfirm={doDelete}
                    onCancel={() => setConfirmDelete(false)}
                />
            )}
        </div>
    )
}

function EmptyState({hasEntries}: {hasEntries: boolean}) {
    const {t} = useTranslation()
    return (
        <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center text-neutral-500">
            <KeyRound className="size-10 text-neutral-300 dark:text-neutral-600" aria-hidden/>
            <p>{t(hasEntries ? 'entries.selectHint' : 'entries.emptyHint')}</p>
            <p className="text-xs">{t('entries.shortcuts')}</p>
        </div>
    )
}

interface Shortcuts {
    search: () => void
    newEntry: () => void
    edit: () => void
    remove: () => void
    lock: () => void
}

/** Ctrl+F search, Ctrl+N new, Ctrl+E edit, Del delete (outside inputs), Ctrl+L lock. */
function useShortcuts(actions: Shortcuts) {
    const ref = useRef(actions)
    useEffect(() => {
        ref.current = actions
    })
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (document.querySelector('[role=alertdialog]')) return
            const key = e.key.toLowerCase()
            const ctrl = e.ctrlKey || e.metaKey
            const target = e.target as HTMLElement | null
            const typing = !!target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
            const a = ref.current
            if (ctrl && key === 'f') a.search()
            else if (ctrl && key === 'n') a.newEntry()
            else if (ctrl && key === 'e') a.edit()
            else if (ctrl && key === 'l') a.lock()
            else if (key === 'delete' && !typing) a.remove()
            else return
            e.preventDefault()
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [])
}
