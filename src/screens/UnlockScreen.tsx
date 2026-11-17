import {type FormEvent, useCallback, useEffect, useRef, useState} from 'react'
import {ScanFace} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Alert, Button, Card, Field, PasswordInput, TextInput} from '../components/ui'
import {api, isAppError} from '../lib/api'
import {errorMessage, throttleDelay} from '../lib/errors'
import {useCountdown} from '../lib/useCountdown'
import {useSession} from '../stores/session'

/**
 * Hello outcomes that need no message: the user chose the password, a dialog is
 * already open, or Hello was turned off meanwhile (the button disappears).
 */
const QUIET_HELLO_ERRORS = new Set(['hello_canceled', 'hello_busy', 'no_such_slot'])

/**
 * Password unlock (`mode="password"`) or recovery-key unlock (`mode="recovery"`).
 * With Windows Hello on, the password screen offers Hello first and starts it once
 * per lock as soon as the window is visible and focused; the password form stays
 * usable the whole time.
 */
export function UnlockScreen({mode}: {mode: 'password' | 'recovery'}) {
    const {t} = useTranslation()
    const go = useSession((s) => s.go)
    const lockReason = useSession((s) => s.lockReason)
    const hello = useSession((s) => s.hello)
    const refreshHello = useSession((s) => s.refreshHello)
    const [secret, setSecret] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<unknown>(null)
    const [retryAt, setRetryAt] = useState<number | null>(null)
    const waitSeconds = useCountdown(retryAt)
    const recovery = mode === 'recovery'
    const showHello = !recovery && hello.enrolled && !hello.broken
    const [helloBusy, setHelloBusy] = useState(false)
    const [helloError, setHelloError] = useState<unknown>(null)
    const passwordRef = useRef<HTMLInputElement>(null)
    // Results that arrive after leaving the screen (password won the race) are ignored.
    const mounted = useRef(true)
    useEffect(() => {
        mounted.current = true
        return () => {
            mounted.current = false
        }
    }, [])

    const unlockHello = useCallback(async () => {
        setHelloBusy(true)
        setHelloError(null)
        try {
            await api.unlockHello()
            if (mounted.current) go('unlocked')
        } catch (err) {
            if (!mounted.current) return
            if (isAppError(err) && (err.code === 'hello_not_found' || err.code === 'no_such_slot')) {
                // Hides the Hello button; a dead slot is removed after the password unlock.
                await refreshHello().catch(() => undefined)
            }
            if (!(isAppError(err) && QUIET_HELLO_ERRORS.has(err.code))) setHelloError(err)
            passwordRef.current?.focus()
        } finally {
            if (mounted.current) setHelloBusy(false)
        }
    }, [go, refreshHello])

    useAutoHello(showHello, unlockHello)

    // The Hello state may have changed since the last check (e.g. turned on before this lock).
    useEffect(() => {
        if (!recovery) refreshHello().catch((e) => console.error('vault_status failed', e))
    }, [recovery, refreshHello])

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
            {!recovery && lockReason && lockReason !== 'manual' && lockReason !== 'exit' && (
                <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t(`lockReason.${lockReason}`)}</p>
            )}
            {helloError !== null && <Alert tone="warning">{errorMessage(t, helloError)}</Alert>}
            {showHello && (
                <>
                    <Button className="w-full py-3" busy={helloBusy} onClick={() => void unlockHello()}>
                        {!helloBusy && <ScanFace className="size-5" aria-hidden/>}
                        {helloBusy ? t('unlock.helloWaiting') : t('unlock.hello')}
                    </Button>
                    <p className="my-4 text-center text-xs text-neutral-500">{t('unlock.orPassword')}</p>
                </>
            )}
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
                        <PasswordInput
                            ref={passwordRef}
                            autoFocus
                            value={secret}
                            onChange={(e) => setSecret(e.target.value)}
                        />
                    )}
                </Field>
                {message && <Alert>{message}</Alert>}
                <Button
                    type="submit"
                    variant={showHello ? 'secondary' : 'primary'}
                    className="w-full"
                    busy={busy}
                    disabled={!secret || waitSeconds > 0}
                >
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

/**
 * Starts Windows Hello once per mount (= once per lock) when it is available and
 * the window is visible and focused. A window locked while hidden (tray, minimized)
 * prompts when it is shown again, not in the background. Cancelling does not
 * prompt again; the Hello button does.
 */
function useAutoHello(enabled: boolean, start: () => void) {
    const done = useRef(false)
    useEffect(() => {
        if (!enabled || done.current) return
        const tryStart = () => {
            if (done.current || document.visibilityState !== 'visible' || !document.hasFocus()) return
            done.current = true
            start()
        }
        tryStart()
        if (done.current) return
        document.addEventListener('visibilitychange', tryStart)
        window.addEventListener('focus', tryStart)
        return () => {
            document.removeEventListener('visibilitychange', tryStart)
            window.removeEventListener('focus', tryStart)
        }
    }, [enabled, start])
}
