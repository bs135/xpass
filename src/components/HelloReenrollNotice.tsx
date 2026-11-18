import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {api} from '../lib/api'
import {useSession} from '../stores/session'
import {useSync} from '../stores/sync'
import {HelloEnablePrompt} from './HelloEnablePrompt'
import {Alert, Button} from './ui'

/**
 * Shown after Windows Hello was turned off by a VaultKey rotation or a Hello reset:
 * offers to set it up again until the user does it or picks "Not now".
 */
export function HelloReenrollNotice() {
    const {t} = useTranslation()
    const suggested = useSession((s) => s.hello.reenrollSuggested)
    const refreshHello = useSession((s) => s.refreshHello)
    // Setting Hello up before a pending key change is resolved would wrap the old key
    // and be dropped again: the sync banner comes first, this notice after it.
    const keyChangePending = useSync((s) => s.status.phase === 'needsPassword')
    const [prompt, setPrompt] = useState(false)

    useEffect(() => {
        refreshHello().catch((e) => console.error('vault_status failed', e))
    }, [refreshHello])

    if (!suggested || keyChangePending) return null

    const dismiss = () => {
        api.helloDismissReenroll()
            .then(refreshHello)
            .catch((e) => console.error('hello_dismiss_reenroll failed', e))
    }

    return (
        <>
            <Alert tone="warning">
                <p className="font-medium">{t('hello.reenrollTitle')}</p>
                <p className="mb-2">{t('hello.reenrollMessage')}</p>
                <div className="flex gap-2">
                    <Button onClick={() => setPrompt(true)}>{t('hello.reenroll')}</Button>
                    <Button variant="secondary" onClick={dismiss}>
                        {t('hello.notNow')}
                    </Button>
                </div>
            </Alert>
            {prompt && <HelloEnablePrompt onClose={() => setPrompt(false)}/>}
        </>
    )
}
