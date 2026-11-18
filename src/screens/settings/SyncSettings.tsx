import {useEffect, useState} from 'react'
import {CheckCircle2, ExternalLink, RefreshCw} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Alert, Button, Field, PasswordInput, TextInput} from '../../components/ui'
import {api, type ConnectionReport, type SyncConfig, type SyncTarget} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {blockingProblem, canSave} from '../../lib/sync'
import {syncText} from '../../lib/syncStatus'
import {useNow} from '../../lib/useNow'
import {useSync} from '../../stores/sync'
import {useToast} from '../../stores/toast'

/**
 * Settings → Sync: GitHub repository, branch, path and token. The token is typed
 * here once, sent to Rust and stored in Windows Credential Manager; it is never
 * read back. All GitHub requests are made by Rust.
 */
export function SyncSettings() {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    const [config, setConfig] = useState<SyncConfig | null>(null)
    const [editing, setEditing] = useState(false)
    const [confirmDisconnect, setConfirmDisconnect] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const refreshStatus = useSync((s) => s.refresh)

    useEffect(() => {
        api.syncGetConfig()
            .then(setConfig)
            .catch((e) => setError(errorMessage(t, e)))
    }, [t])

    const disconnect = async () => {
        setConfirmDisconnect(false)
        setError(null)
        try {
            setConfig(await api.syncDisconnect())
            refreshStatus()
            setEditing(false)
            show(t('sync.disconnected'))
        } catch (e) {
            setError(errorMessage(t, e))
        }
    }

    if (!config) {
        return error ? <Alert>{error}</Alert> : <p className="text-sm text-neutral-500">…</p>
    }

    return (
        <>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('sync.intro')}</p>
            {error && <Alert>{error}</Alert>}
            {config.repo && !editing ? (
                <>
                    <dl className="mb-4 grid grid-cols-[8rem_1fr] gap-y-1 text-sm">
                        <dt className="text-neutral-500">{t('sync.repo')}</dt>
                        <dd className="break-all">{config.repo}</dd>
                        <dt className="text-neutral-500">{t('sync.branch')}</dt>
                        <dd className="break-all">{config.branch || t('sync.defaultBranch')}</dd>
                        <dt className="text-neutral-500">{t('sync.path')}</dt>
                        <dd className="break-all">{config.path}</dd>
                        <dt className="text-neutral-500">{t('sync.token')}</dt>
                        <dd>{config.hasToken ? t('sync.tokenStored') : t('sync.tokenMissing')}</dd>
                    </dl>
                    {!config.hasToken && <Alert tone="warning">{t('sync.needsToken')}</Alert>}
                    {config.enabled && config.hasToken && <SyncStatusLine/>}
                    <div className="flex flex-wrap gap-2">
                        <Button variant="secondary" onClick={() => setEditing(true)}>
                            {t('sync.change')}
                        </Button>
                        <Button variant="danger" onClick={() => setConfirmDisconnect(true)}>
                            {t('sync.disconnect')}
                        </Button>
                    </div>
                </>
            ) : (
                <SyncForm
                    config={config}
                    onSaved={(c) => {
                        setConfig(c)
                        refreshStatus()
                        setEditing(false)
                        show(t('sync.saved'))
                    }}
                    onCancel={config.repo ? () => setEditing(false) : undefined}
                />
            )}
            <TokenGuide/>
            {confirmDisconnect && (
                <ConfirmDialog
                    title={t('sync.disconnectTitle')}
                    message={t('sync.disconnectMessage')}
                    confirmLabel={t('sync.disconnect')}
                    onConfirm={() => void disconnect()}
                    onCancel={() => setConfirmDisconnect(false)}
                />
            )}
        </>
    )
}

function SyncStatusLine() {
    const {t} = useTranslation()
    const status = useSync((s) => s.status)
    const syncNow = useSync((s) => s.syncNow)
    const show = useToast((s) => s.show)
    const now = useNow()
    return (
        <div className="mb-4 flex flex-wrap items-center justify-between gap-2 rounded-md bg-neutral-50 px-3 py-2 text-sm dark:bg-neutral-900">
            <span role="status">{syncText(t, status, now)}</span>
            <Button
                variant="secondary"
                busy={status.phase === 'syncing'}
                onClick={() => syncNow().catch((e) => show(errorMessage(t, e), 4000))}
            >
                {status.phase !== 'syncing' && <RefreshCw className="size-4" aria-hidden/>}
                {t('sync.syncNow')}
            </Button>
        </div>
    )
}

function SyncForm({
    config,
    onSaved,
    onCancel,
}: {
    config: SyncConfig
    onSaved: (config: SyncConfig) => void
    onCancel?: () => void
}) {
    const {t} = useTranslation()
    const [target, setTarget] = useState<SyncTarget>({repo: config.repo, branch: config.branch, path: config.path})
    const [token, setToken] = useState('')
    // With a stored token the field stays closed unless the user replaces it.
    const [replaceToken, setReplaceToken] = useState(!config.hasToken)
    const [report, setReport] = useState<ConnectionReport | null>(null)
    const [publicOk, setPublicOk] = useState(false)
    const [busy, setBusy] = useState<'test' | 'save' | null>(null)
    const [error, setError] = useState<string | null>(null)

    const typedToken = replaceToken ? token.trim() : ''
    const tokenReady = !replaceToken || typedToken.length > 0

    // Any change invalidates the last test.
    const edit = (patch: Partial<SyncTarget>) => {
        setTarget((v) => ({...v, ...patch}))
        setReport(null)
        setPublicOk(false)
    }

    const test = async () => {
        setBusy('test')
        setError(null)
        setReport(null)
        try {
            setReport(await api.syncTestConnection(target, typedToken || null))
        } catch (e) {
            setError(errorMessage(t, e))
        } finally {
            setBusy(null)
        }
    }

    const save = async () => {
        setBusy('save')
        setError(null)
        try {
            let saved = await api.syncConfigure(target)
            if (typedToken) saved = await api.syncSetToken(typedToken)
            setToken('')
            onSaved(saved)
        } catch (e) {
            setError(errorMessage(t, e))
        } finally {
            setBusy(null)
        }
    }

    const problem = report && blockingProblem(report)

    return (
        <form
            className="mb-4"
            onSubmit={(e) => {
                e.preventDefault()
                void test()
            }}
        >
            <Field label={t('sync.repo')}>
                <TextInput
                    value={target.repo}
                    placeholder={t('sync.repoPlaceholder')}
                    onChange={(e) => edit({repo: e.target.value})}
                />
            </Field>
            <div className="grid gap-x-4 sm:grid-cols-2">
                <Field label={t('sync.branch')}>
                    <TextInput
                        value={target.branch}
                        placeholder={t('sync.defaultBranch')}
                        onChange={(e) => edit({branch: e.target.value})}
                    />
                </Field>
                <Field label={t('sync.path')}>
                    <TextInput value={target.path} onChange={(e) => edit({path: e.target.value})}/>
                </Field>
            </div>
            {replaceToken ? (
                <Field label={t('sync.token')}>
                    <PasswordInput
                        value={token}
                        placeholder="github_pat_…"
                        onChange={(e) => {
                            setToken(e.target.value)
                            setReport(null)
                        }}
                    />
                </Field>
            ) : (
                <div className="mb-3 flex flex-wrap items-center justify-between gap-2 text-sm">
                    <span>{t('sync.tokenStored')}</span>
                    <Button variant="link" onClick={() => setReplaceToken(true)}>
                        {t('sync.replaceToken')}
                    </Button>
                </div>
            )}

            {error && <Alert>{error}</Alert>}
            {report && (
                <>
                    {problem ? (
                        <Alert>{t(`sync.report.${problem}`, {branch: report.branch})}</Alert>
                    ) : (
                        <p className="mb-3 flex items-start gap-2 text-sm text-emerald-700 dark:text-emerald-400">
                            <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden/>
                            {t(`sync.report.${report.remote}`, {branch: report.branch})}
                        </p>
                    )}
                    {!problem && !report.private && (
                        <>
                            <Alert tone="warning">{t('sync.report.public')}</Alert>
                            <label className="mb-3 flex items-center gap-2 text-sm">
                                <input
                                    type="checkbox"
                                    className="accent-emerald-600"
                                    checked={publicOk}
                                    onChange={(e) => setPublicOk(e.target.checked)}
                                />
                                {t('sync.report.publicAck')}
                            </label>
                        </>
                    )}
                </>
            )}

            <div className="flex flex-wrap gap-2">
                <Button
                    type="submit"
                    variant="secondary"
                    busy={busy === 'test'}
                    disabled={busy !== null || !target.repo.trim() || !tokenReady}
                >
                    {t('sync.test')}
                </Button>
                <Button
                    busy={busy === 'save'}
                    disabled={busy !== null || !canSave(report, publicOk)}
                    onClick={() => void save()}
                >
                    {t('actions.save')}
                </Button>
                {onCancel && (
                    <Button variant="secondary" disabled={busy !== null} onClick={onCancel}>
                        {t('actions.cancel')}
                    </Button>
                )}
            </div>
            <p className="mt-2 text-xs text-neutral-500">{t('sync.testFirst')}</p>
        </form>
    )
}

function TokenGuide() {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    return (
        <details className="mt-4 rounded-md border border-neutral-200 p-3 text-sm dark:border-neutral-800">
            <summary className="cursor-pointer font-medium">{t('sync.guide.title')}</summary>
            <ol className="mt-2 list-decimal space-y-1 pl-5 text-neutral-700 dark:text-neutral-300">
                {(t('sync.guide.steps', {returnObjects: true}) as string[]).map((line, i) => (
                    <li key={i}>{line}</li>
                ))}
            </ol>
            <p className="mt-2 text-neutral-600 dark:text-neutral-400">{t('sync.guide.note')}</p>
            <Button
                variant="secondary"
                className="mt-3"
                onClick={() => api.syncOpenTokenPage().catch((e) => show(errorMessage(t, e), 4000))}
            >
                <ExternalLink className="size-4" aria-hidden/>
                {t('sync.guide.open')}
            </Button>
        </details>
    )
}
