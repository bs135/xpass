import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {HelloEnablePrompt} from '../../components/HelloEnablePrompt'
import {HelloReenrollNotice} from '../../components/HelloReenrollNotice'
import {Alert, Button} from '../../components/ui'
import {api} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useSession} from '../../stores/session'
import {useSync} from '../../stores/sync'
import {useToast} from '../../stores/toast'

/** Settings → Security: turn Windows Hello on or off for this vault. */
export function HelloSettings() {
    const {t} = useTranslation()
    const enrolled = useSession((s) => s.hello.enrolled)
    const refreshHello = useSession((s) => s.refreshHello)
    const keyChangePending = useSync((s) => s.status.phase === 'needsPassword')
    const show = useToast((s) => s.show)
    // null while asking Windows whether Hello is set up.
    const [supported, setSupported] = useState<boolean | null>(null)
    const [prompt, setPrompt] = useState<'enable' | 'disable' | null>(null)
    const [error, setError] = useState<string | null>(null)

    useEffect(() => {
        api.helloStatus()
            .then((s) => setSupported(s.supported))
            .catch(() => setSupported(false))
    }, [])

    const disable = async () => {
        setPrompt(null)
        setError(null)
        try {
            await api.helloDisable()
            await refreshHello()
            show(t('hello.disabled'))
        } catch (e) {
            setError(errorMessage(t, e))
        }
    }

    return (
        <>
            <h3 className="mb-1 font-medium">{t('hello.title')}</h3>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('hello.intro')}</p>
            <HelloReenrollNotice/>
            {error && <Alert>{error}</Alert>}
            {supported === null ? (
                <p className="text-sm text-neutral-500">…</p>
            ) : enrolled ? (
                <>
                    {!supported && <Alert tone="warning">{t('hello.unavailableEnrolled')}</Alert>}
                    <div className="flex flex-wrap items-center justify-between gap-3">
                        <p className="text-sm">{t('hello.stateOn')}</p>
                        <Button variant="secondary" onClick={() => setPrompt('disable')}>
                            {t('hello.turnOff')}
                        </Button>
                    </div>
                </>
            ) : supported ? (
                <>
                    <div className="flex flex-wrap items-center justify-between gap-3">
                        <p className="text-sm">{t('hello.stateOff')}</p>
                        <Button disabled={keyChangePending} onClick={() => setPrompt('enable')}>
                            {t('hello.turnOn')}
                        </Button>
                    </div>
                    {keyChangePending && (
                        <p className="mt-2 text-sm text-neutral-600 dark:text-neutral-400">{t('hello.keyChangePending')}</p>
                    )}
                </>
            ) : (
                <p className="text-sm text-neutral-600 dark:text-neutral-400">{t('hello.unavailable')}</p>
            )}

            {prompt === 'enable' && <HelloEnablePrompt onClose={() => setPrompt(null)}/>}
            {prompt === 'disable' && (
                <ConfirmDialog
                    title={t('hello.turnOffTitle')}
                    message={t('hello.turnOffMessage')}
                    confirmLabel={t('hello.turnOff')}
                    onConfirm={() => void disable()}
                    onCancel={() => setPrompt(null)}
                />
            )}
        </>
    )
}
