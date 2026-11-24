import {type FormEvent, useState} from 'react'
import {Download, Fingerprint} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Modal} from '../../components/Modal'
import {NewPasswordForm} from '../../components/NewPasswordForm'
import {Alert, Button, Field, PasswordInput} from '../../components/ui'
import {api, type ExportAuth, type ExportFormat, type ExportResult} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useSession} from '../../stores/session'
import {useToast} from '../../stores/toast'

type Step =
    | {stage: 'warn'}
    | {stage: 'kdbxPassword'}
    | {stage: 'auth'; format: ExportFormat; kdbxPassword: string | null}
    | {stage: 'done'; format: ExportFormat; result: ExportResult}

/**
 * Settings → Import & export → Export. Order (plan I7): warning or export password →
 * master password / Windows Hello → save dialog (in Rust) → result. The file is
 * built and written in Rust; the UI only gets its name and the entry count.
 */
export function ExportSettings() {
    const {t} = useTranslation()
    const [step, setStep] = useState<Step | null>(null)
    const close = () => setStep(null)

    return (
        <>
            <h3 className="mb-1 font-medium">{t('export.title')}</h3>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('export.intro')}</p>
            <div className="flex flex-wrap gap-2">
                <Button variant="secondary" onClick={() => setStep({stage: 'kdbxPassword'})}>
                    <Download className="size-4" aria-hidden/>
                    {t('export.kdbx')}
                </Button>
                <Button variant="secondary" onClick={() => setStep({stage: 'warn'})}>
                    <Download className="size-4" aria-hidden/>
                    {t('export.csv')}
                </Button>
            </div>

            {step?.stage === 'warn' && (
                <CsvWarning onContinue={() => setStep({stage: 'auth', format: 'csv', kdbxPassword: null})} onClose={close}/>
            )}
            {step?.stage === 'kdbxPassword' && (
                <KdbxPassword
                    onContinue={(kdbxPassword) => setStep({stage: 'auth', format: 'kdbx', kdbxPassword})}
                    onClose={close}
                />
            )}
            {step?.stage === 'auth' && (
                <AuthPrompt
                    format={step.format}
                    kdbxPassword={step.kdbxPassword}
                    onDone={(result) => setStep(result ? {stage: 'done', format: step.format, result} : null)}
                    onClose={close}
                />
            )}
            {step?.stage === 'done' && <Result format={step.format} result={step.result} onClose={close}/>}
        </>
    )
}

function CsvWarning({onContinue, onClose}: {onContinue: () => void; onClose: () => void}) {
    const {t} = useTranslation()
    const [understood, setUnderstood] = useState(false)
    return (
        <Modal title={t('export.csvTitle')} onClose={onClose}>
            <Alert tone="warning">{t('export.csvWarning')}</Alert>
            <ul className="mb-4 list-disc space-y-1 pl-5 text-sm text-neutral-600 dark:text-neutral-400">
                {(t('export.csvRules', {returnObjects: true}) as string[]).map((line, i) => (
                    <li key={i}>{line}</li>
                ))}
            </ul>
            <label className="mb-4 flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    className="accent-emerald-600"
                    checked={understood}
                    onChange={(e) => setUnderstood(e.target.checked)}
                />
                {t('export.understand')}
            </label>
            <div className="flex justify-end gap-2">
                <Button variant="secondary" onClick={onClose}>
                    {t('actions.cancel')}
                </Button>
                <Button variant="danger" disabled={!understood} onClick={onContinue}>
                    {t('export.continue')}
                </Button>
            </div>
        </Modal>
    )
}

function KdbxPassword({onContinue, onClose}: {onContinue: (password: string) => void; onClose: () => void}) {
    const {t} = useTranslation()
    return (
        <Modal title={t('export.kdbxTitle')} onClose={onClose}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('export.kdbxExplain')}</p>
            <NewPasswordForm
                label={t('export.kdbxPassword')}
                submitLabel={t('export.continue')}
                busyLabel={t('export.continue')}
                error={null}
                onSubmit={async (password) => onContinue(password)}
            />
        </Modal>
    )
}

function AuthPrompt({
    format,
    kdbxPassword,
    onDone,
    onClose,
}: {
    format: ExportFormat
    kdbxPassword: string | null
    onDone: (result: ExportResult | null) => void
    onClose: () => void
}) {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    const hello = useSession((s) => s.hello)
    const [password, setPassword] = useState('')
    const [busy, setBusy] = useState<'password' | 'hello' | null>(null)
    const [error, setError] = useState<string | null>(null)

    const run = async (auth: ExportAuth, which: 'password' | 'hello') => {
        setBusy(which)
        setError(null)
        try {
            const result = await api.exportVault(format, auth, kdbxPassword)
            if (!result) show(t('export.canceled'))
            onDone(result)
        } catch (e) {
            setError(errorMessage(t, e))
            setBusy(null)
        }
    }

    const submit = (e: FormEvent) => {
        e.preventDefault()
        if (!password || busy) return
        void run({method: 'password', password}, 'password')
    }

    return (
        <Modal title={t('export.authTitle')} onClose={onClose}>
            <form onSubmit={submit}>
                <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('export.authExplain')}</p>
                <Field label={t('settings.currentPassword')}>
                    <PasswordInput autoFocus value={password} onChange={(e) => setPassword(e.target.value)}/>
                </Field>
                {error && <Alert>{error}</Alert>}
                <div className="flex flex-wrap justify-end gap-2">
                    {hello.enrolled && !hello.broken && (
                        <Button
                            variant="secondary"
                            busy={busy === 'hello'}
                            disabled={busy !== null}
                            onClick={() => void run({method: 'hello'}, 'hello')}
                        >
                            <Fingerprint className="size-4" aria-hidden/>
                            {t('export.useHello')}
                        </Button>
                    )}
                    <Button variant="secondary" onClick={onClose}>
                        {t('actions.cancel')}
                    </Button>
                    <Button type="submit" busy={busy === 'password'} disabled={!password || busy !== null}>
                        {t('export.chooseFile')}
                    </Button>
                </div>
            </form>
        </Modal>
    )
}

function Result({format, result, onClose}: {format: ExportFormat; result: ExportResult; onClose: () => void}) {
    const {t} = useTranslation()
    return (
        <Modal title={t('export.doneTitle')} onClose={onClose}>
            <p className="mb-3 text-sm">{t('export.done', {count: result.count, name: result.fileName})}</p>
            {format === 'csv' ? (
                <Alert tone="warning">{t('export.csvAfter')}</Alert>
            ) : (
                <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('export.kdbxAfter')}</p>
            )}
            <div className="flex justify-end">
                <Button onClick={onClose}>{t('actions.close')}</Button>
            </div>
        </Modal>
    )
}
