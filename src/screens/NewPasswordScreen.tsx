import {useState} from 'react'
import {useTranslation} from 'react-i18next'
import {NewPasswordForm} from '../components/NewPasswordForm'
import {Alert, Button, Card} from '../components/ui'
import {api} from '../lib/api'
import {errorMessage} from '../lib/errors'
import {useSession} from '../stores/session'

/**
 * After a recovery-key unlock: a new master password is required, then a new
 * Emergency Kit is offered (the old recovery key keeps working otherwise).
 */
export function NewPasswordScreen() {
    const {t} = useTranslation()
    const {go, showKit} = useSession()
    const [step, setStep] = useState<'password' | 'kit'>('password')
    const [error, setError] = useState<string | null>(null)
    const [busy, setBusy] = useState(false)

    const save = async (password: string) => {
        setError(null)
        try {
            await api.changePassword(null, password)
            setStep('kit')
        } catch (e) {
            setError(errorMessage(t, e))
        }
    }

    const newKit = async () => {
        setBusy(true)
        setError(null)
        try {
            const {recoveryKey} = await api.newRecoveryKey(null)
            showKit(recoveryKey)
        } catch (e) {
            setError(errorMessage(t, e))
        } finally {
            setBusy(false)
        }
    }

    if (step === 'kit') {
        return (
            <Card title={t('newPassword.kitTitle')}>
                <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('newPassword.kitIntro')}</p>
                {error && <Alert>{error}</Alert>}
                <div className="flex gap-2">
                    <Button variant="secondary" onClick={() => go('unlocked')}>
                        {t('newPassword.kitSkip')}
                    </Button>
                    <Button className="flex-1" busy={busy} onClick={newKit}>
                        {t('newPassword.kitCreate')}
                    </Button>
                </div>
            </Card>
        )
    }

    return (
        <Card title={t('newPassword.title')}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('newPassword.intro')}</p>
            <NewPasswordForm
                submitLabel={t('newPassword.submit')}
                busyLabel={t('newPassword.saving')}
                onSubmit={save}
                error={error}
            />
        </Card>
    )
}
