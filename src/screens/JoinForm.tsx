import {type FormEvent, useState} from 'react'
import {CheckCircle2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Alert, Button, Field, PasswordInput, TextInput} from '../components/ui'
import {api, type ConnectionReport, type SyncTarget} from '../lib/api'
import {errorMessage} from '../lib/errors'
import {canJoin, joinProblem} from '../lib/sync'
import {useSession} from '../stores/session'
import {useSync} from '../stores/sync'
import {TokenGuide} from './settings/SyncSettings'

const DEFAULT_PATH = 'xpass/vault.xpv'

/**
 * "Use my vault from GitHub": repository and token, test connection, then the
 * master password of that vault. Rust downloads and opens it; the token is stored
 * in Windows Credential Manager and never comes back.
 */
export function JoinForm() {
    const {t} = useTranslation()
    const go = useSession((s) => s.go)
    const refreshHello = useSession((s) => s.refreshHello)
    const refreshSync = useSync((s) => s.refresh)
    const [target, setTarget] = useState<SyncTarget>({repo: '', branch: '', path: DEFAULT_PATH})
    const [token, setToken] = useState('')
    const [password, setPassword] = useState('')
    const [report, setReport] = useState<ConnectionReport | null>(null)
    const [publicOk, setPublicOk] = useState(false)
    const [busy, setBusy] = useState<'test' | 'join' | null>(null)
    const [error, setError] = useState<string | null>(null)

    // Any change invalidates the last test.
    const edit = (patch: Partial<SyncTarget>) => {
        setTarget((v) => ({...v, ...patch}))
        setReport(null)
        setPublicOk(false)
    }

    const test = async (e: FormEvent) => {
        e.preventDefault()
        setBusy('test')
        setError(null)
        setReport(null)
        try {
            setReport(await api.syncTestConnection(target, token.trim()))
        } catch (err) {
            setError(errorMessage(t, err))
        } finally {
            setBusy(null)
        }
    }

    const join = async (e: FormEvent) => {
        e.preventDefault()
        if (!password) return
        setBusy('join')
        setError(null)
        try {
            await api.syncJoin(target, token.trim(), password)
            setPassword('')
            setToken('')
            refreshSync()
            await refreshHello().catch(() => undefined)
            go('joined')
        } catch (err) {
            setError(errorMessage(t, err))
            setBusy(null)
        }
    }

    const problem = report && joinProblem(report)
    const ready = canJoin(report, publicOk)

    return (
        <>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('join.intro')}</p>
            <form onSubmit={test}>
                <Field label={t('sync.repo')}>
                    <TextInput
                        autoFocus
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
                {report &&
                    (problem ? (
                        <Alert>{t(`sync.report.${problem}`, {branch: report.branch})}</Alert>
                    ) : (
                        <p className="mb-3 flex items-start gap-2 text-sm text-emerald-700 dark:text-emerald-400">
                            <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden/>
                            {t('sync.report.found')}
                        </p>
                    ))}
                {report && !problem && !report.private && (
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
                {!ready && (
                    <>
                        {error && <Alert>{error}</Alert>}
                        <Button
                            type="submit"
                            variant="secondary"
                            className="w-full"
                            busy={busy === 'test'}
                            disabled={busy !== null || !target.repo.trim() || !token.trim()}
                        >
                            {t('sync.test')}
                        </Button>
                    </>
                )}
            </form>
            {ready && (
                <form onSubmit={join}>
                    <Field label={t('join.password')}>
                        <PasswordInput autoFocus value={password} onChange={(e) => setPassword(e.target.value)}/>
                    </Field>
                    {error && <Alert>{error}</Alert>}
                    <Button type="submit" className="w-full" busy={busy === 'join'} disabled={!password}>
                        {busy === 'join' ? t('join.joining') : t('join.submit')}
                    </Button>
                </form>
            )}
            <TokenGuide/>
        </>
    )
}
