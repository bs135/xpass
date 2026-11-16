import {CheckCircle2} from 'lucide-react'
import {useToast} from '../stores/toast'

export function Toast() {
    const message = useToast((s) => s.message)
    if (!message) return null
    return (
        <div
            role="status"
            className="pointer-events-none fixed bottom-10 left-1/2 z-40 flex -translate-x-1/2 items-center gap-2 rounded-full bg-neutral-900 px-4 py-2 text-sm text-white shadow-lg dark:bg-neutral-100 dark:text-neutral-900"
        >
            <CheckCircle2 className="size-4 text-emerald-400 dark:text-emerald-600" aria-hidden/>
            {message}
        </div>
    )
}
