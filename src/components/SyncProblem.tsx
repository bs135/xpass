import {useState} from 'react'
import {useTranslation} from 'react-i18next'
import {api} from '../lib/api'
import {errorMessage} from '../lib/errors'
import {PasswordPrompt} from '../screens/settings/PasswordPrompt'
import {useEntries} from '../stores/entries'
import {useSession} from '../stores/session'
import {useSync} from '../stores/sync'
import {useToast} from '../stores/toast'
import {Alert, Button} from './ui'

/**
 * Sync problems only the user can fix, with the action that fixes them, plus the
 * one-time "password changed on another device" notice. On the entry list and in
 * Settings → Sync (`inSettings`: no "open settings" buttons).
 */
export function SyncProblem({inSettings = false}: {inSettings?: boolean}) {
    const {t} = useTranslation()
    const status = useSync((s) => s.status)
    const refreshSync = useSync((s) => s.refresh)
    const go = useSession((s) => s.go)
    const refreshHello = useSession((s) => s.refreshHello)
    const show = useToast((s) => s.show)
    const [prompt, setPrompt] = useState<'password' | 'replace' | null>(null)

    const afterChange = async (message: string) => {
        setPrompt(null)
        refreshSync()
        await refreshHello().catch(() => undefined)
        await useEntries.getState().refresh().catch(() => undefined)
        show(message)
    }

    const resolve = async (password: string) => {
        useSync.getState().set(await api.syncResolveKeyChange(password))
        await afterChange(t('syncProblem.resumed'))
    }

    const replace = async (password: string) => {
        await api.syncReplaceLocal(password)
        await afterChange(t('syncProblem.replaced'))
    }

    const dismiss = () => {
        api.syncDismissPasswordChanged()
            .then(refreshSync)
            .catch((e) => show(errorMessage(t, e), 4000))
    }

    const settingsButton = (label: string) =>
        !inSettings && (
            <Button variant="secondary" onClick={() => go('settings')}>
                {label}
            </Button>
        )

    return (
        <>
            {status.passwordChanged && (
                <Alert tone="warning">
                    <p className="font-medium">{t('syncProblem.passwordChangedTitle')}</p>
                    <p className="mb-2">{t('syncProblem.passwordChanged')}</p>
                    <Button variant="secondary" onClick={dismiss}>
                        {t('actions.ok')}
                    </Button>
                </Alert>
            )}
            {status.phase === 'tokenInvalid' && (
                <Alert tone="warning">
                    <p className="font-medium">{t('syncProblem.tokenTitle')}</p>
                    <p className={inSettings ? undefined : 'mb-2'}>
                        {t(inSettings ? 'syncProblem.tokenInSettings' : 'syncProblem.token')}
                    </p>
                    {settingsButton(t('syncProblem.openSettings'))}
                </Alert>
            )}
            {status.phase === 'needsPassword' && (
                <Alert tone="warning">
                    <p className="font-medium">{t('syncProblem.keyTitle')}</p>
                    <p className="mb-2">{t('syncProblem.key')}</p>
                    <Button onClick={() => setPrompt('password')}>{t('syncProblem.enterPassword')}</Button>
                </Alert>
            )}
            {status.phase === 'differentVault' && (
                <Alert tone="warning">
                    <p className="font-medium">{t('syncProblem.otherTitle')}</p>
                    <p className="mb-2">{t(inSettings ? 'syncProblem.otherInSettings' : 'syncProblem.other')}</p>
                    <div className="flex flex-wrap gap-2">
                        {settingsButton(t('syncProblem.otherPath'))}
                        <Button variant="danger" onClick={() => setPrompt('replace')}>
                            {t('syncProblem.replace')}
                        </Button>
                    </div>
                </Alert>
            )}

            {prompt === 'password' && (
                <PasswordPrompt
                    title={t('syncProblem.keyTitle')}
                    label={t('syncProblem.keyPassword')}
                    confirmLabel={t('syncProblem.continue')}
                    onConfirm={resolve}
                    onClose={() => setPrompt(null)}
                >
                    <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('syncProblem.keyExplain')}</p>
                </PasswordPrompt>
            )}
            {prompt === 'replace' && (
                <PasswordPrompt
                    title={t('syncProblem.replace')}
                    label={t('syncProblem.replacePassword')}
                    confirmLabel={t('syncProblem.replaceConfirm')}
                    danger
                    onConfirm={replace}
                    onClose={() => setPrompt(null)}
                >
                    <ul className="mb-4 list-disc space-y-1 pl-5 text-sm text-neutral-600 dark:text-neutral-400">
                        {(t('syncProblem.replaceExplain', {returnObjects: true}) as string[]).map((line, i) => (
                            <li key={i}>{line}</li>
                        ))}
                    </ul>
                </PasswordPrompt>
            )}
        </>
    )
}
