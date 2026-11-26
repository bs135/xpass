import {useEffect, useMemo, useRef, useState} from 'react'
import {KeyRound} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {HelloReenrollNotice} from '../../components/HelloReenrollNotice'
import {SyncProblem} from '../../components/SyncProblem'
import {Alert} from '../../components/ui'
import {copyField} from '../../lib/copy'
import {errorMessage} from '../../lib/errors'
import {flaggedIds} from '../../lib/health'
import {filterEntries} from '../../lib/search'
import {useEntries} from '../../stores/entries'
import {useSession} from '../../stores/session'
import {EntryForm} from './EntryForm'
import {EntryList} from './EntryList'
import {EntryView} from './EntryView'

/** Two-pane entry manager: list + search on the left, details or form on the right. */
export function EntriesScreen() {
    const {t} = useTranslation()
    const {entries, query, favoritesOnly, healthOnly, health, selectedId, detail, mode, load, remove} = useEntries()
    const lock = useSession((s) => s.lock)
    const searchRef = useRef<HTMLInputElement>(null)
    const [confirmDelete, setConfirmDelete] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const visible = useMemo(() => {
        const found = filterEntries(entries, query, favoritesOnly)
        if (!healthOnly) return found
        if (!health) return []
        const flagged = flaggedIds(health)
        return found.filter((e) => flagged.has(e.id))
    }, [entries, query, favoritesOnly, healthOnly, health])

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
        copy: (kind) => {
            const s = useEntries.getState()
            if (!s.detail || s.mode !== 'view') return false
            if (kind === 'password' && !s.detail.hasPassword) return false
            if (kind === 'username' && !s.detail.username) return false
            void copyField(t, s.detail.id, {kind})
            return true
        },
    })

    const doDelete = () => {
        setConfirmDelete(false)
        if (selectedId) remove(selectedId).catch((e) => setError(errorMessage(t, e)))
    }

    return (
        <div className="flex min-h-0 flex-1">
            <EntryList visible={visible} searchRef={searchRef}/>
            <section className="min-w-0 flex-1 overflow-auto">
                <div className="px-4 pt-4 empty:hidden">
                    <SyncProblem/>
                    <HelloReenrollNotice/>
                </div>
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
                    <EntryView key={detail.id} detail={detail} onDelete={() => setConfirmDelete(true)}/>
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
            <p className="text-xs">{t('entries.shortcutsCopy')}</p>
        </div>
    )
}

interface Shortcuts {
    search: () => void
    newEntry: () => void
    edit: () => void
    remove: () => void
    lock: () => void
    /** Returns false when there is nothing to copy (the key then keeps its default). */
    copy: (kind: 'username' | 'password') => boolean
}

/**
 * Ctrl+F search, Ctrl+N new, Ctrl+E edit, Del delete (outside inputs), Ctrl+L lock,
 * Ctrl+B copy username, Ctrl+C copy password (only when no text is selected and the
 * focus is not in a field, so normal copying keeps working).
 */
function useShortcuts(actions: Shortcuts) {
    const ref = useRef(actions)
    useEffect(() => {
        ref.current = actions
    })
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (document.querySelector('[role=alertdialog], [role=dialog]')) return
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
            else if (ctrl && key === 'b') {
                if (!a.copy('username')) return
            } else if (ctrl && key === 'c' && !typing && !window.getSelection()?.toString()) {
                if (!a.copy('password')) return
            } else return
            e.preventDefault()
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [])
}
