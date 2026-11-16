import {type FormEvent, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {Alert, Button, Card, Field, PasswordInput, TextInput} from '../components/ui'
import {api, isAppError} from '../lib/api'
import {errorMessage, throttleDelay} from '../lib/errors'
import {useCountdown} from '../lib/useCountdown'
import {useSession} from '../stores/session'

/** Password unlock (`mode="password"`) or recovery-key unlock (`mode="recovery"`). */
export function UnlockScreen({mode}: {mode: 'password' | 'recovery'}) {
    const {t} = useTranslation()
    const go = useSession((s) => s.go)
    const [secret, setSecret] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<unknown>(null)
    const [retryAt, setRetryAt] = useState<number | null>(null)
    const waitSeconds = useCountdown(retryAt)
    const recovery = mode === 'recovery'

    const submit = async (e: FormEvent) => {
        e.preventDefault()
        if (!secret || busy || waitSeconds > 0) return
        setBusy(true)
        setError(null)
        try {
            if (recovery) {
                await api.unlockRecovery(secret)
                go('newPassword')
            } else {
                await api.unlockPassword(secret)
                go('unlocked')
            }
            setSecret('')
        } catch (err) {
            setError(err)
            const delay = throttleDelay(err)
            setRetryAt(delay === null ? null : Date.now() + delay)
        } finally {
            setBusy(false)
        }
    }

    const message =
        waitSeconds > 0
            ? t('errors.throttled', {seconds: waitSeconds})
            : error !== null && throttleDelay(error) === null
              ? recovery && isAppError(error) && error.code === 'bad_credentials'
                  ? t('errors.wrong_recovery_key')
                  : errorMessage(t, error)
              : null

    return (
        <Card title={t(recovery ? 'recovery.title' : 'unlock.title')}>
            {recovery && <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('recovery.intro')}</p>}
            <form onSubmit={submit}>
                <Field label={t(recovery ? 'recovery.key' : 'unlock.password')}>
                    {recovery ? (
                        <TextInput
                            autoFocus
                            className="font-mono uppercase"
                            placeholder="XXXXX-XXXXX-…"
                            value={secret}
                            onChange={(e) => setSecret(e.target.value)}
                        />
                    ) : (
                        <PasswordInput autoFocus value={secret} onChange={(e) => setSecret(e.target.value)}/>
                    )}
                </Field>
                {message && <Alert>{message}</Alert>}
                <Button type="submit" className="w-full" busy={busy} disabled={!secret || waitSeconds > 0}>
                    {busy ? t('unlock.unlocking') : t('unlock.submit')}
                </Button>
            </form>
            <div className="mt-4 text-center text-sm">
                <Button variant="link" onClick={() => go(recovery ? 'unlock' : 'recovery')}>
                    {t(recovery ? 'recovery.back' : 'unlock.useRecovery')}
                </Button>
            </div>
        </Card>
    )
}
