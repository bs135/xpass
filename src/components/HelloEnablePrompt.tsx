import {useTranslation} from 'react-i18next'
import {api} from '../lib/api'
import {PasswordPrompt} from '../screens/settings/PasswordPrompt'
import {useSession} from '../stores/session'
import {useToast} from '../stores/toast'

/**
 * Turns Windows Hello on: asks for the master password, then Windows shows its own
 * Hello dialog. Errors (wrong password, Hello cancelled) stay in the prompt.
 */
export function HelloEnablePrompt({onClose}: {onClose: () => void}) {
    const {t} = useTranslation()
    const refreshHello = useSession((s) => s.refreshHello)
    const show = useToast((s) => s.show)

    const enable = async (password: string) => {
        await api.helloEnable(password)
        await refreshHello().catch(() => undefined)
        show(t('hello.enabled'))
        onClose()
    }

    return (
        <PasswordPrompt title={t('hello.turnOn')} confirmLabel={t('hello.turnOnConfirm')} onConfirm={enable} onClose={onClose}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('hello.turnOnExplain')}</p>
        </PasswordPrompt>
    )
}
