import {useEffect, useRef} from 'react'
import {useTranslation} from 'react-i18next'
import {Button} from './ui'

interface Props {
    title: string
    message: string
    confirmLabel: string
    onConfirm: () => void
    onCancel: () => void
}

/** Modal confirmation. Esc cancels; the confirm button has focus, so Enter confirms. */
export function ConfirmDialog({title, message, confirmLabel, onConfirm, onCancel}: Props) {
    const {t} = useTranslation()
    const confirmRef = useRef<HTMLButtonElement>(null)

    useEffect(() => {
        confirmRef.current?.focus()
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                onCancel()
            }
        }
        window.addEventListener('keydown', onKey, {capture: true})
        return () => window.removeEventListener('keydown', onKey, {capture: true})
    }, [onCancel])

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6" onClick={onCancel}>
            <div
                role="alertdialog"
                aria-modal="true"
                aria-labelledby="confirm-title"
                className="w-full max-w-sm rounded-xl border border-neutral-200 bg-white p-5 shadow-lg dark:border-neutral-700 dark:bg-neutral-900"
                onClick={(e) => e.stopPropagation()}
            >
                <h2 id="confirm-title" className="mb-2 text-lg font-semibold">
                    {title}
                </h2>
                <p className="mb-5 text-sm text-neutral-600 dark:text-neutral-400">{message}</p>
                <div className="flex justify-end gap-2">
                    <Button variant="secondary" onClick={onCancel}>
                        {t('actions.cancel')}
                    </Button>
                    <Button ref={confirmRef} variant="danger" onClick={onConfirm}>
                        {confirmLabel}
                    </Button>
                </div>
            </div>
        </div>
    )
}
