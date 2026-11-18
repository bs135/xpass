import {useEffect, useState} from 'react'
import {CheckCircle2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {HelloEnablePrompt} from '../components/HelloEnablePrompt'
import {Button, Card} from '../components/ui'
import {api} from '../lib/api'
import {useSession} from '../stores/session'

/**
 * After "Use my vault from GitHub": the existing Emergency Kit still applies (no new
 * one), and Windows Hello can be set up for this PC.
 */
export function JoinedScreen() {
    const {t} = useTranslation()
    const go = useSession((s) => s.go)
    const enrolled = useSession((s) => s.hello.enrolled)
    const [helloSupported, setHelloSupported] = useState(false)
    const [prompt, setPrompt] = useState(false)

    useEffect(() => {
        api.helloStatus()
            .then((s) => setHelloSupported(s.supported))
            .catch(() => setHelloSupported(false))
    }, [])

    return (
        <Card title={t('join.doneTitle')}>
            <p className="mb-3 flex items-start gap-2 text-sm text-emerald-700 dark:text-emerald-400">
                <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden/>
                {t('join.done')}
            </p>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('join.kitReminder')}</p>
            {helloSupported && !enrolled && (
                <div className="mb-4 rounded-md bg-neutral-50 p-3 text-sm dark:bg-neutral-900">
                    <p className="mb-2">{t('join.helloOffer')}</p>
                    <Button variant="secondary" onClick={() => setPrompt(true)}>
                        {t('hello.turnOn')}
                    </Button>
                </div>
            )}
            <Button className="w-full" onClick={() => go('unlocked')}>
                {t('join.continue')}
            </Button>
            {prompt && <HelloEnablePrompt onClose={() => setPrompt(false)}/>}
        </Card>
    )
}
