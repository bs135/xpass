import {type ReactNode, useEffect, useState} from 'react'
import {ChevronDown, ChevronRight, Copy, ExternalLink, Eye, EyeOff, Pencil, RotateCcw, Star, Trash2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Alert, Button} from '../../components/ui'
import {api, type EntryDetail, type FieldRef, type HistoryItemView} from '../../lib/api'
import {copyField} from '../../lib/copy'
import {errorMessage} from '../../lib/errors'
import {useEntries} from '../../stores/entries'
import {TotpCode} from './TotpCode'

/** Reveal hides again after this long, or when the window loses focus. */
const REVEAL_MS = 15_000

/** Identifies what is revealed: one secret at a time. */
function refKey(field: FieldRef): string {
    switch (field.kind) {
        case 'history':
            return `history:${field.index}:${field.at}`
        case 'field':
        case 'url':
        case 'totp':
            return `${field.kind}:${field.index}`
        default:
            return field.kind
    }
}

/**
 * Read-only details. A secret (password, secret custom field, old password) is
 * fetched only for an explicit reveal and kept in local state until it is hidden
 * again; copying goes through Rust.
 */
export function EntryView({detail, onDelete}: {detail: EntryDetail; onDelete: () => void}) {
    const {t, i18n} = useTranslation()
    const {startEdit, toggleFavorite} = useEntries()
    const [error, setError] = useState<string | null>(null)
    const [revealed, setRevealed] = useState<{key: string; value: string} | null>(null)

    useEffect(() => {
        if (revealed === null) return
        const hide = () => setRevealed(null)
        const timer = window.setTimeout(hide, REVEAL_MS)
        window.addEventListener('blur', hide)
        return () => {
            window.clearTimeout(timer)
            window.removeEventListener('blur', hide)
        }
    }, [revealed])

    // The key includes the entry and its version: another entry, or a new version of
    // this one (save, sync, "use"), hides what was revealed.
    const version = `${detail.id}:${detail.updatedAt}`
    const keyOf = (field: FieldRef) => `${version}:${refKey(field)}`
    const shown = (field: FieldRef) => (revealed?.key === keyOf(field) ? revealed.value : null)

    const toggleReveal = (field: FieldRef) => {
        const key = keyOf(field)
        if (revealed?.key === key) {
            setRevealed(null)
            return
        }
        setError(null)
        api.revealSecret(detail.id, field)
            .then((value) => setRevealed({key, value}))
            .catch((e) => setError(errorMessage(t, e)))
    }

    const revealButton = (field: FieldRef) => (
        <IconButton
            icon={shown(field) === null ? Eye : EyeOff}
            title={t(shown(field) === null ? 'actions.show' : 'actions.hide')}
            onClick={() => toggleReveal(field)}
        />
    )

    const copy = (field: FieldRef) => void copyField(t, detail.id, field)

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
                <Row
                    label={t('entries.username')}
                    actions={
                        detail.username && (
                            <IconButton icon={Copy} title={t('copy.username')} onClick={() => copy({kind: 'username'})}/>
                        )
                    }
                >
                    {detail.username || <Empty/>}
                </Row>
                <Row
                    label={t('entries.password')}
                    actions={
                        detail.hasPassword && (
                            <>
                                {revealButton({kind: 'password'})}
                                <IconButton icon={Copy} title={t('copy.password')} onClick={() => copy({kind: 'password'})}/>
                            </>
                        )
                    }
                >
                    {detail.hasPassword ? <Secret value={shown({kind: 'password'})}/> : <Empty/>}
                </Row>
                <Row label={t('entries.urls')}>
                    {detail.urls.length === 0 ? (
                        <Empty/>
                    ) : (
                        <ul className="space-y-1">
                            {detail.urls.map((url, i) => (
                                <li key={i} className="flex items-center gap-2">
                                    <span className="min-w-0 flex-1 truncate">{url}</span>
                                    <IconButton icon={Copy} title={t('copy.url')} onClick={() => copy({kind: 'url', index: i})}/>
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
                {detail.fields.map((f, index) => {
                    const field: FieldRef = {kind: 'field', index}
                    if (f.totp) {
                        // The code, not the secret (V4); the secret is revealed in the edit form.
                        return (
                            <Row
                                key={index}
                                label={f.name || t('totp.label')}
                                actions={
                                    <IconButton
                                        icon={Copy}
                                        title={t('copy.totp')}
                                        onClick={() => copy({kind: 'totp', index})}
                                    />
                                }
                            >
                                <TotpCode entryId={detail.id} index={index} version={version}/>
                            </Row>
                        )
                    }
                    return (
                        <Row
                            key={index}
                            label={f.name || t('entries.customField')}
                            actions={
                                <>
                                    {f.secret && revealButton(field)}
                                    <IconButton icon={Copy} title={t('copy.field')} onClick={() => copy(field)}/>
                                </>
                            }
                        >
                            {f.secret ? (
                                <Secret value={shown(field)}/>
                            ) : f.value ? (
                                <span className="whitespace-pre-wrap">{f.value}</span>
                            ) : (
                                <Empty/>
                            )}
                        </Row>
                    )
                })}
            </dl>
            {detail.historyCount > 0 && (
                <PasswordHistory
                    detail={detail}
                    shown={shown}
                    revealButton={revealButton}
                    onCopy={copy}
                    onError={setError}
                />
            )}
            <p className="mt-3 text-xs text-neutral-500">
                {t('entries.updated', {date: new Date(detail.updatedAt).toLocaleString(i18n.language)})}
            </p>
        </div>
    )
}

function PasswordHistory({
    detail,
    shown,
    revealButton,
    onCopy,
    onError,
}: {
    detail: EntryDetail
    shown: (field: FieldRef) => string | null
    revealButton: (field: FieldRef) => ReactNode
    onCopy: (field: FieldRef) => void
    onError: (message: string | null) => void
}) {
    const {t, i18n} = useTranslation()
    const [open, setOpen] = useState(false)
    const [items, setItems] = useState<HistoryItemView[] | null>(null)
    const [using, setUsing] = useState<HistoryItemView | null>(null)

    // Loaded when opened, and again when the entry changes (save, sync, "use").
    useEffect(() => {
        if (!open) return
        let current = true
        api.listHistory(detail.id)
            .then((list) => {
                if (current) setItems(list)
            })
            .catch((e) => {
                if (current) onError(errorMessage(t, e))
            })
        return () => {
            current = false
        }
    }, [open, detail.id, detail.updatedAt, onError, t])

    const use = async (item: HistoryItemView) => {
        setUsing(null)
        onError(null)
        try {
            await api.useHistoryPassword(detail.id, item.index, item.at)
            await useEntries.getState().refresh()
        } catch (e) {
            onError(errorMessage(t, e))
        }
    }

    const Chevron = open ? ChevronDown : ChevronRight
    return (
        <section className="mt-4">
            <button
                type="button"
                aria-expanded={open}
                onClick={() => setOpen((o) => !o)}
                className="flex items-center gap-1 text-sm font-medium hover:text-emerald-600"
            >
                <Chevron className="size-4" aria-hidden/>
                {t('history.title', {count: detail.historyCount})}
            </button>
            {open && items && (
                <ul className="mt-2 divide-y divide-neutral-100 rounded-lg border border-neutral-200 text-sm dark:divide-neutral-800 dark:border-neutral-800">
                    {items.map((item) => {
                        const field: FieldRef = {kind: 'history', index: item.index, at: item.at}
                        return (
                            <li
                                key={`${item.index}:${item.at}`}
                                className="grid grid-cols-[8rem_1fr_auto] items-start gap-3 px-4 py-3"
                            >
                                <div className="text-xs text-neutral-500">
                                    {new Date(item.at).toLocaleString(i18n.language, {
                                        dateStyle: 'medium',
                                        timeStyle: 'short',
                                    })}
                                    {item.note && (
                                        <span className="mt-1 block w-fit rounded bg-neutral-100 px-1.5 py-0.5 dark:bg-neutral-800">
                                            {t(`history.note.${item.note}`, {defaultValue: item.note})}
                                        </span>
                                    )}
                                </div>
                                <div data-selectable className="min-w-0 break-words">
                                    {item.hasPassword ? <Secret value={shown(field)}/> : <Empty/>}
                                    {item.username && (
                                        <p className="text-xs text-neutral-500">
                                            {t('history.username', {username: item.username})}
                                        </p>
                                    )}
                                </div>
                                <div className="-my-1 flex gap-0.5">
                                    {item.hasPassword && (
                                        <>
                                            {revealButton(field)}
                                            <IconButton icon={Copy} title={t('copy.password')} onClick={() => onCopy(field)}/>
                                            <IconButton icon={RotateCcw} title={t('history.use')} onClick={() => setUsing(item)}/>
                                        </>
                                    )}
                                </div>
                            </li>
                        )
                    })}
                </ul>
            )}
            {using && (
                <ConfirmDialog
                    title={t('history.useTitle')}
                    message={t('history.useMessage')}
                    confirmLabel={t('history.use')}
                    onConfirm={() => void use(using)}
                    onCancel={() => setUsing(null)}
                />
            )}
        </section>
    )
}

/** Masked secret, or its value while revealed. */
function Secret({value}: {value: string | null}) {
    return value === null ? (
        <span className="font-mono tracking-widest">••••••••••</span>
    ) : (
        <span className="font-mono break-all">{value}</span>
    )
}

function Row({label, actions, children}: {label: string; actions?: ReactNode; children: ReactNode}) {
    return (
        <div className="grid grid-cols-[8rem_1fr_auto] items-start gap-3 px-4 py-3 text-sm">
            <dt className="text-neutral-500">{label}</dt>
            <dd data-selectable className="min-w-0 break-words">
                {children}
            </dd>
            <div className="-my-1 flex gap-0.5">{actions}</div>
        </div>
    )
}

function IconButton({icon: Icon, title, onClick}: {icon: typeof Copy; title: string; onClick: () => void}) {
    return (
        <button
            type="button"
            title={title}
            aria-label={title}
            onClick={onClick}
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 hover:text-emerald-600 dark:hover:bg-neutral-800"
        >
            <Icon className="size-4" aria-hidden/>
        </button>
    )
}

function Empty() {
    return <span className="text-neutral-400">—</span>
}
