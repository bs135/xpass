import {type FormEvent, useEffect, useState} from 'react'
import {Fingerprint, History} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Modal} from '../../components/Modal'
import {Alert, Button, Field, PasswordInput} from '../../components/ui'
import {api, type ExportAuth, type HistoryPreview, type RewriteReport} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useSession} from '../../stores/session'
import {useSync} from '../../stores/sync'

/**
 * "Remove old versions from GitHub" (v0.7 plan U10): replaces the sync branch with
 * one commit holding the current files. Rust checks the preconditions (sync on,
 * unlocked, synced, no local changes), re-authenticates the user and talks to GitHub.
 */
export function RemoveOldVersions({variant = 'secondary'}: {variant?: 'secondary' | 'danger'}) {
    const {t} = useTranslation()
    const [open, setOpen] = useState(false)
    return (
        <>
            <Button variant={variant} onClick={() => setOpen(true)}>
                <History className="size-4" aria-hidden/>
                {t('sync.history.button')}
            </Button>
            {open && <HistoryDialog onClose={() => setOpen(false)}/>}
        </>
    )
}

function HistoryDialog({onClose}: {onClose: () => void}) {
    const {t} = useTranslation()
    const hello = useSession((s) => s.hello)
    const refreshSync = useSync((s) => s.refresh)
    const [preview, setPreview] = useState<HistoryPreview | null>(null)
    const [report, setReport] = useState<RewriteReport | null>(null)
    const [password, setPassword] = useState('')
    const [busy, setBusy] = useState<'password' | 'hello' | null>(null)
    const [error, setError] = useState<string | null>(null)

    useEffect(() => {
        api.syncHistoryPreview()
            .then(setPreview)
            .catch((e) => setError(errorMessage(t, e)))
    }, [t])

    const run = async (auth: ExportAuth, which: 'password' | 'hello') => {
        setBusy(which)
        setError(null)
        try {
            setReport(await api.syncRewriteHistory(auth))
            setPassword('')
        } catch (e) {
            setError(errorMessage(t, e))
        } finally {
            setBusy(null)
            refreshSync()
        }
    }

    const submit = (e: FormEvent) => {
        e.preventDefault()
        if (!password || busy) return
        void run({method: 'password', password}, 'password')
    }

    if (report) {
        return (
            <Modal title={t('sync.history.title')} onClose={onClose}>
                <p className="mb-4 text-sm" role="status">
                    {report.rewritten
                        ? t('sync.history.done', {branch: preview?.branch ?? ''})
                        : t('sync.history.single', {branch: preview?.branch ?? ''})}
                </p>
                <div className="flex justify-end">
                    <Button onClick={onClose}>{t('actions.close')}</Button>
                </div>
            </Modal>
        )
    }

    return (
        <Modal title={t('sync.history.title')} onClose={onClose}>
            {!preview ? (
                <>
                    {error ? <Alert>{error}</Alert> : <p className="mb-4 text-sm text-neutral-500">…</p>}
                    <div className="flex justify-end">
                        <Button variant="secondary" onClick={onClose}>
                            {t('actions.close')}
                        </Button>
                    </div>
                </>
            ) : preview.singleCommit ? (
                <>
                    <p className="mb-4 text-sm">{t('sync.history.single', {branch: preview.branch})}</p>
                    <div className="flex justify-end">
                        <Button onClick={onClose}>{t('actions.close')}</Button>
                    </div>
                </>
            ) : (
                <form onSubmit={submit}>
                    <ul className="mb-3 list-disc space-y-1 pl-5 text-sm text-neutral-600 dark:text-neutral-400">
                        {(t('sync.history.explain', {returnObjects: true, branch: preview.branch}) as string[]).map(
                            (line, i) => (
                                <li key={i}>{line}</li>
                            ),
                        )}
                    </ul>
                    <Alert tone="warning">
                        <p>{t('sync.history.limits')}</p>
                    </Alert>
                    {preview.otherFiles > 0 && (
                        <Alert tone="warning">
                            <p>{t('sync.history.otherFiles', {count: preview.otherFiles})}</p>
                            <ul className="mt-1 list-disc pl-5 font-mono text-xs">
                                {preview.otherPaths.map((p) => (
                                    <li key={p} className="break-all">
                                        {p}
                                    </li>
                                ))}
                                {preview.otherFiles > preview.otherPaths.length && (
                                    <li>{t('sync.history.more', {count: preview.otherFiles - preview.otherPaths.length})}</li>
                                )}
                            </ul>
                        </Alert>
                    )}
                    <Field label={t('settings.currentPassword')}>
                        <PasswordInput autoFocus value={password} onChange={(e) => setPassword(e.target.value)}/>
                    </Field>
                    {error && <Alert>{error}</Alert>}
                    <div className="flex flex-wrap justify-end gap-2">
                        {hello.enrolled && !hello.broken && (
                            <Button
                                variant="secondary"
                                busy={busy === 'hello'}
                                disabled={busy !== null}
                                onClick={() => void run({method: 'hello'}, 'hello')}
                            >
                                <Fingerprint className="size-4" aria-hidden/>
                                {t('sync.history.useHello')}
                            </Button>
                        )}
                        <Button variant="secondary" onClick={onClose}>
                            {t('actions.cancel')}
                        </Button>
                        <Button
                            type="submit"
                            variant="danger"
                            busy={busy === 'password'}
                            disabled={!password || busy !== null}
                        >
                            {t('sync.history.confirm')}
                        </Button>
                    </div>
                </form>
            )}
        </Modal>
    )
}
