import {type Ref} from 'react'
import {Plus, Search, Star} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import type {EntrySummary} from '../../lib/api'
import {useEntries} from '../../stores/entries'

interface Props {
    visible: EntrySummary[]
    searchRef: Ref<HTMLInputElement>
}

export function EntryList({visible, searchRef}: Props) {
    const {t} = useTranslation()
    const {entries, selectedId, query, favoritesOnly, select, startNew, setQuery, setFavoritesOnly} = useEntries()

    return (
        <aside className="flex w-80 shrink-0 flex-col border-r border-neutral-200 dark:border-neutral-800">
            <div className="flex items-center gap-1.5 border-b border-neutral-200 p-2 dark:border-neutral-800">
                <div className="relative flex-1">
                    <Search className="pointer-events-none absolute top-2.5 left-2.5 size-4 text-neutral-400" aria-hidden/>
                    <input
                        ref={searchRef}
                        type="search"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Escape') setQuery('')
                            if (e.key === 'Enter' && visible[0]) void select(visible[0].id)
                        }}
                        placeholder={t('entries.search')}
                        aria-label={t('entries.search')}
                        autoComplete="off"
                        spellCheck={false}
                        className="w-full rounded-md border border-neutral-300 bg-white py-1.5 pr-2 pl-8 text-sm outline-none focus:border-emerald-500 dark:border-neutral-700 dark:bg-neutral-900"
                    />
                </div>
                <button
                    type="button"
                    aria-pressed={favoritesOnly}
                    title={t('entries.favoritesOnly')}
                    onClick={() => setFavoritesOnly(!favoritesOnly)}
                    className={clsx(
                        'rounded-md p-2 hover:bg-neutral-100 dark:hover:bg-neutral-800',
                        favoritesOnly ? 'text-amber-500' : 'text-neutral-500',
                    )}
                >
                    <Star className="size-4" fill={favoritesOnly ? 'currentColor' : 'none'} aria-hidden/>
                </button>
                <button
                    type="button"
                    title={`${t('entries.new')} (Ctrl+N)`}
                    onClick={startNew}
                    className="rounded-md bg-emerald-600 p-2 text-white hover:bg-emerald-700"
                >
                    <Plus className="size-4" aria-hidden/>
                </button>
            </div>
            <ul className="flex-1 overflow-auto" role="listbox" aria-label={t('entries.list')}>
                {visible.map((e) => (
                    <li key={e.id} role="option" aria-selected={e.id === selectedId}>
                        <button
                            type="button"
                            onClick={() => void select(e.id)}
                            className={clsx(
                                'flex w-full items-center gap-2 border-b border-neutral-100 px-3 py-2 text-left dark:border-neutral-800/60',
                                e.id === selectedId
                                    ? 'bg-emerald-50 dark:bg-emerald-950/40'
                                    : 'hover:bg-neutral-50 dark:hover:bg-neutral-800/50',
                            )}
                        >
                            <div className="min-w-0 flex-1">
                                <div className="truncate font-medium">{e.title}</div>
                                <div className="truncate text-xs text-neutral-500">{e.username || e.urlHost || ' '}</div>
                            </div>
                            {e.favorite && <Star className="size-3.5 shrink-0 text-amber-500" fill="currentColor" aria-label={t('entries.favorite')}/>}
                        </button>
                    </li>
                ))}
            </ul>
            <div className="border-t border-neutral-200 px-3 py-1.5 text-xs text-neutral-500 dark:border-neutral-800">
                {entries.length === 0
                    ? t('entries.emptyVault')
                    : visible.length === entries.length
                      ? t('entries.count', {count: entries.length})
                      : t('entries.countFiltered', {shown: visible.length, count: entries.length})}
            </div>
        </aside>
    )
}
