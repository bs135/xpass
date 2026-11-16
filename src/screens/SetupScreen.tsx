import {useState} from 'react'
import {useTranslation} from 'react-i18next'
import {NewPasswordForm} from '../components/NewPasswordForm'
import {Card} from '../components/ui'
import {api} from '../lib/api'
import {errorMessage} from '../lib/errors'
import {useSession} from '../stores/session'

export function SetupScreen() {
    const {t} = useTranslation()
    const showKit = useSession((s) => s.showKit)
    const [error, setError] = useState<string | null>(null)

    const create = async (password: string) => {
        setError(null)
        try {
            const {recoveryKey} = await api.createVault(password)
            showKit(recoveryKey)
        } catch (e) {
            setError(errorMessage(t, e))
        }
    }

    return (
        <Card title={t('setup.title')}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('setup.intro')}</p>
            <NewPasswordForm submitLabel={t('setup.submit')} busyLabel={t('setup.creating')} onSubmit={create} error={error}/>
        </Card>
    )
}
