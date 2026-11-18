import {useState} from 'react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {NewPasswordForm} from '../components/NewPasswordForm'
import {Card} from '../components/ui'
import {api} from '../lib/api'
import {errorMessage} from '../lib/errors'
import {useSession} from '../stores/session'
import {JoinForm} from './JoinForm'

/** First start: create a new vault, or use the vault already on GitHub (another PC). */
export function SetupScreen() {
    const {t} = useTranslation()
    const showKit = useSession((s) => s.showKit)
    const [mode, setMode] = useState<'create' | 'join'>('create')
    const [error, setError] = useState<string | null>(null)

    const create = async (password: string) => {
        setError(null)
        try {
            const {recoveryKey} = await api.createVault(password)
            showKit(recoveryKey)
        } catch (e) {
            setError(errorMessage(t, e))
            throw e
        }
    }

    return (
        <Card title={mode === 'create' ? t('setup.title') : t('join.title')}>
            <div role="tablist" className="mb-4 grid grid-cols-2 gap-1 rounded-md bg-neutral-100 p-1 text-sm dark:bg-neutral-800">
                {(['create', 'join'] as const).map((m) => (
                    <button
                        key={m}
                        type="button"
                        role="tab"
                        aria-selected={mode === m}
                        onClick={() => setMode(m)}
                        className={clsx(
                            'rounded px-2 py-1.5 font-medium',
                            mode === m
                                ? 'bg-white shadow-sm dark:bg-neutral-950'
                                : 'text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100',
                        )}
                    >
                        {m === 'create' ? t('setup.createTab') : t('setup.joinTab')}
                    </button>
                ))}
            </div>
            {mode === 'create' ? (
                <>
                    <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('setup.intro')}</p>
                    <NewPasswordForm
                        submitLabel={t('setup.submit')}
                        busyLabel={t('setup.creating')}
                        onSubmit={create}
                        error={error}
                    />
                </>
            ) : (
                <JoinForm/>
            )}
        </Card>
    )
}
