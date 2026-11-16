import {type ReactNode, useState} from 'react'
import {ExternalLink, Pencil, Star, Trash2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Alert, Button} from '../../components/ui'
import {api, type EntryDetail} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useEntries} from '../../stores/entries'

/** Read-only details. The password is never loaded here (reveal/copy come in T7). */
export function EntryView({detail, onDelete}: {detail: EntryDetail; onDelete: () => void}) {
    const {t, i18n} = useTranslation()
    const {startEdit, toggleFavorite} = useEntries()
    const [error, setError] = useState<string | null>(null)

    const run = (action: Promise<void>) => {
        setError(null)
        action.catch((e) => setError(errorMessage(t, e)))
    }

    return (
        <div className="mx-auto w-full max-w-2xl p-6">
            <div className="mb-6 flex items-start gap-3">
                <h1 data-selectable className="min-w-0 flex-1 text-2xl font-semibold break-words">
                    {detail.title}
                </h1>
                <button
                    type="button"
                    aria-pressed={detail.favorite}
                    title={t(detail.favorite ? 'entries.unfavorite' : 'entries.markFavorite')}
                    onClick={() => run(toggleFavorite(detail.id, !detail.favorite))}
                    className="rounded-md p-2 text-amber-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
                >
                    <Star className="size-5" fill={detail.favorite ? 'currentColor' : 'none'} aria-hidden/>
                </button>
                <Button variant="secondary" title="Ctrl+E" onClick={startEdit}>
                    <Pencil className="size-4" aria-hidden/>
                    {t('actions.edit')}
                </Button>
                <Button variant="secondary" title="Del" onClick={onDelete}>
                    <Trash2 className="size-4" aria-hidden/>
                    {t('actions.delete')}
                </Button>
            </div>
            {error && <Alert>{error}</Alert>}
            <dl className="divide-y divide-neutral-100 rounded-lg border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
                <Row label={t('entries.username')}>{detail.username || <Empty/>}</Row>
                <Row label={t('entries.password')}>
                    {detail.hasPassword ? <span className="font-mono tracking-widest">••••••••••</span> : <Empty/>}
                </Row>
                <Row label={t('entries.urls')}>
                    {detail.urls.length === 0 ? (
                        <Empty/>
                    ) : (
                        <ul className="space-y-1">
                            {detail.urls.map((url, i) => (
                                <li key={i} className="flex items-center gap-2">
                                    <span className="min-w-0 flex-1 truncate">{url}</span>
                                    <button
                                        type="button"
                                        title={t('entries.openUrl')}
                                        onClick={() => run(api.openUrl(detail.id, i))}
                                        className="rounded p-1 text-neutral-500 hover:bg-neutral-100 hover:text-emerald-600 dark:hover:bg-neutral-800"
                                    >
                                        <ExternalLink className="size-4" aria-hidden/>
                                    </button>
                                </li>
                            ))}
                        </ul>
                    )}
                </Row>
                <Row label={t('entries.tags')}>
                    {detail.tags.length === 0 ? (
                        <Empty/>
                    ) : (
                        <div className="flex flex-wrap gap-1.5">
                            {detail.tags.map((tag) => (
                                <span key={tag} className="rounded-full bg-neutral-100 px-2 py-0.5 text-xs dark:bg-neutral-800">
                                    {tag}
                                </span>
                            ))}
                        </div>
                    )}
                </Row>
                <Row label={t('entries.notes')}>
                    {detail.notes ? <p className="whitespace-pre-wrap">{detail.notes}</p> : <Empty/>}
                </Row>
            </dl>
            <p className="mt-3 text-xs text-neutral-500">
                {t('entries.updated', {date: new Date(detail.updatedAt).toLocaleString(i18n.language)})}
            </p>
        </div>
    )
}

function Row({label, children}: {label: string; children: ReactNode}) {
    return (
        <div className="grid grid-cols-[8rem_1fr] gap-3 px-4 py-3 text-sm">
            <dt className="text-neutral-500">{label}</dt>
            <dd data-selectable className="min-w-0 break-words">
                {children}
            </dd>
        </div>
    )
}

function Empty() {
    return <span className="text-neutral-400">—</span>
}
