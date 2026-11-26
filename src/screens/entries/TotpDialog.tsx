import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {Modal} from '../../components/Modal'
import {Alert, Button, Field, PasswordInput, TextInput} from '../../components/ui'
import {api, type TotpPreview} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {formatCode} from '../../lib/totp'

/**
 * "Add one-time password" (v0.8 plan V5): paste an otpauth:// URI or a base32
 * secret; Rust checks it, normalizes it and shows the current code, so a wrong
 * secret shows up before it is saved. `onAdd` gets the normalized URI.
 */
export function TotpDialog({
    issuer: defaultIssuer,
    account: defaultAccount,
    onAdd,
    onClose,
}: {
    issuer: string
    account: string
    onAdd: (uri: string) => void
    onClose: () => void
}) {
    const {t} = useTranslation()
    const [secret, setSecret] = useState('')
    const [issuer, setIssuer] = useState(defaultIssuer)
    const [account, setAccount] = useState(defaultAccount)
    // The result belongs to the inputs it was computed for; any edit makes it stale.
    const [result, setResult] = useState<{key: string; preview?: TotpPreview; error?: string} | null>(null)
    const key = JSON.stringify([secret, issuer, account])
    const current = result?.key === key ? result : null
    const preview = current?.preview ?? null
    const error = current?.error ?? null

    useEffect(() => {
        if (!secret.trim()) return
        let live = true
        const timer = window.setTimeout(() => {
            api.totpPreview(secret, issuer, account)
                .then((p) => {
                    if (live) setResult({key, preview: p})
                })
                .catch((e) => {
                    if (live) setResult({key, error: errorMessage(t, e)})
                })
        }, 300)
        return () => {
            live = false
            window.clearTimeout(timer)
        }
    }, [key, secret, issuer, account, t])

    return (
        <Modal title={t('totp.addTitle')} onClose={onClose}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('totp.addExplain')}</p>
            <Field label={t('totp.secret')}>
                <PasswordInput
                    autoFocus
                    value={secret}
                    placeholder="otpauth://totp/…"
                    onChange={(e) => setSecret(e.target.value)}
                />
            </Field>
            <div className="grid gap-x-4 sm:grid-cols-2">
                <Field label={t('totp.issuer')}>
                    <TextInput maxLength={128} value={issuer} onChange={(e) => setIssuer(e.target.value)}/>
                </Field>
                <Field label={t('totp.account')}>
                    <TextInput maxLength={128} value={account} onChange={(e) => setAccount(e.target.value)}/>
                </Field>
            </div>
            {error && <Alert>{error}</Alert>}
            {preview && (
                <p className="mb-4 text-sm" role="status">
                    {t('totp.previewCode')}{' '}
                    <span className="font-mono text-lg tracking-wider">{formatCode(preview.code)}</span>
                    <span className="ml-2 text-neutral-500">{t('totp.compare')}</span>
                </p>
            )}
            <div className="flex justify-end gap-2">
                <Button variant="secondary" onClick={onClose}>
                    {t('actions.cancel')}
                </Button>
                <Button disabled={!preview} onClick={() => preview && onAdd(preview.uri)}>
                    {t('totp.add')}
                </Button>
            </div>
        </Modal>
    )
}
