import {type FormEvent, type ReactNode, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {Modal} from '../../components/Modal'
import {Alert, Button, Field, PasswordInput} from '../../components/ui'
import {errorMessage} from '../../lib/errors'

interface Props {
    title: string
    children?: ReactNode
    confirmLabel: string
    danger?: boolean
    /** Receives the current master password; throw to show the error. */
    onConfirm: (password: string) => Promise<void>
    onClose: () => void
}

/** Asks for the current master password before a sensitive action. */
export function PasswordPrompt({title, children, confirmLabel, danger, onConfirm, onClose}: Props) {
    const {t} = useTranslation()
    const [password, setPassword] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const submit = async (e: FormEvent) => {
        e.preventDefault()
        if (!password || busy) return
        setBusy(true)
        setError(null)
        try {
            await onConfirm(password)
        } catch (err) {
            setError(errorMessage(t, err))
            setBusy(false)
        }
    }

    return (
        <Modal title={title} onClose={onClose}>
            <form onSubmit={submit}>
                {children}
                <Field label={t('settings.currentPassword')}>
                    <PasswordInput autoFocus value={password} onChange={(e) => setPassword(e.target.value)}/>
                </Field>
                {error && <Alert>{error}</Alert>}
                <div className="flex justify-end gap-2">
                    <Button variant="secondary" onClick={onClose}>
                        {t('actions.cancel')}
                    </Button>
                    <Button type="submit" variant={danger ? 'danger' : 'primary'} busy={busy} disabled={!password}>
                        {confirmLabel}
                    </Button>
                </div>
            </form>
        </Modal>
    )
}
