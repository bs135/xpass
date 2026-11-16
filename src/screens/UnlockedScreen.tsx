import {LockOpen} from 'lucide-react'
import {useTranslation} from 'react-i18next'

/** Placeholder until the entry list arrives (T6). */
export function UnlockedScreen() {
    const {t} = useTranslation()
    return (
        <main className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
            <LockOpen className="size-12 text-emerald-600 dark:text-emerald-400" aria-hidden/>
            <h1 className="text-xl font-semibold">{t('unlocked.title')}</h1>
            <p className="max-w-md text-neutral-500 dark:text-neutral-400">{t('unlocked.body')}</p>
        </main>
    )
}
