import {type FormEvent, useCallback, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {MIN_STRENGTH} from '../lib/api'
import {StrengthMeter} from './StrengthMeter'
import {Alert, Button, Field, PasswordInput} from './ui'

interface Props {
    submitLabel: string
    busyLabel: string
    /** Receives the new password; set `error` and throw on failure (fields are kept). */
    onSubmit: (password: string) => Promise<void>
    error: string | null
    /** Label of the first field (default: "Master password"). */
    label?: string
    /** Focus the first field on mount (default true). */
    autoFocus?: boolean
}

/** New master password + confirmation + strength gate (setup and reset). */
export function NewPasswordForm({submitLabel, busyLabel, onSubmit, error, label, autoFocus = true}: Props) {
    const {t} = useTranslation()
    const [password, setPassword] = useState('')
    const [confirm, setConfirm] = useState('')
    const [scored, setScored] = useState<{password: string; score: number} | null>(null)
    const [busy, setBusy] = useState(false)
    const onScore = useCallback((pw: string, score: number) => setScored({password: pw, score}), [])
    // Only trust a score computed for exactly the current password.
    const score = scored?.password === password ? scored.score : null

    const mismatch = confirm.length > 0 && confirm !== password
    const canSubmit = password.length > 0 && password === confirm && score !== null && score >= MIN_STRENGTH

    const submit = async (e: FormEvent) => {
        e.preventDefault()
        if (!canSubmit) return
        setBusy(true)
        try {
            await onSubmit(password)
            // Clear only on success, so a failed attempt keeps what the user typed.
            setPassword('')
            setConfirm('')
            setScored(null)
        } catch {
            // The parent shows the error (`error` prop).
        } finally {
            setBusy(false)
        }
    }

    return (
        <form onSubmit={submit}>
            <Field label={label ?? t('password.new')}>
                <PasswordInput autoFocus={autoFocus} value={password} onChange={(e) => setPassword(e.target.value)}/>
            </Field>
            <StrengthMeter password={password} onScore={onScore}/>
            <Field label={t('password.confirm')}>
                <PasswordInput value={confirm} onChange={(e) => setConfirm(e.target.value)}/>
            </Field>
            {mismatch && <Alert tone="warning">{t('password.mismatch')}</Alert>}
            {score !== null && score < MIN_STRENGTH && <Alert tone="warning">{t('password.tooWeak')}</Alert>}
            {error && <Alert>{error}</Alert>}
            <Button type="submit" className="w-full" disabled={!canSubmit} busy={busy}>
                {busy ? busyLabel : submitLabel}
            </Button>
        </form>
    )
}
