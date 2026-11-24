import {type ReactNode, useEffect} from 'react'
import {createPortal} from 'react-dom'
import {X} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {BackdropDragStrip} from './WindowControls'

/**
 * Simple modal with a title; Esc and the backdrop close it. Rendered into <body> so a
 * modal opened from inside a form cannot submit that form.
 */
export function Modal({
    title,
    onClose,
    wide,
    children,
}: {
    title: string
    onClose: () => void
    /** Wider dialog (tables). */
    wide?: boolean
    children: ReactNode
}) {
    const {t} = useTranslation()
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                onClose()
            }
        }
        window.addEventListener('keydown', onKey, {capture: true})
        return () => window.removeEventListener('keydown', onKey, {capture: true})
    }, [onClose])

    return createPortal(
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6" onClick={onClose}>
            <BackdropDragStrip/>
            <div
                role="dialog"
                aria-modal="true"
                aria-label={title}
                className={`flex max-h-full w-full flex-col ${wide ? 'max-w-3xl' : 'max-w-md'} rounded-xl border border-neutral-200 bg-white py-5 shadow-lg dark:border-neutral-700 dark:bg-neutral-900`}
                onClick={(e) => e.stopPropagation()}
            >
                <div className="mb-4 flex shrink-0 items-center justify-between px-5">
                    <h2 className="text-lg font-semibold">{title}</h2>
                    <button
                        type="button"
                        onClick={onClose}
                        title={t('actions.close')}
                        className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
                    >
                        <X className="size-4" aria-hidden/>
                    </button>
                </div>
                {/* The body scrolls when the dialog is taller than the window (the title
                    stays); `overscroll-contain` keeps the wheel from scrolling the page
                    behind. Its children are plain blocks, so they keep their height. */}
                <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5">{children}</div>
            </div>
        </div>,
        document.body,
    )
}
