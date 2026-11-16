import {type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, useState} from 'react'
import {AlertTriangle, Eye, EyeOff, Loader2} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'

const inputClass =
    'w-full rounded-md border border-neutral-300 bg-white px-3 py-2 outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/30 dark:border-neutral-700 dark:bg-neutral-900'

export function Card({title, children}: {title: string; children: ReactNode}) {
    return (
        <div className="flex flex-1 items-center justify-center overflow-auto p-6">
            <div className="w-full max-w-md rounded-xl border border-neutral-200 bg-white p-6 shadow-sm dark:border-neutral-800 dark:bg-neutral-900/50">
                <h1 className="mb-4 text-xl font-semibold">{title}</h1>
                {children}
            </div>
        </div>
    )
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: 'primary' | 'secondary' | 'link'
    busy?: boolean
}

export function Button({variant = 'primary', busy, disabled, className, children, ...rest}: ButtonProps) {
    return (
        <button
            type="button"
            disabled={disabled || busy}
            className={clsx(
                'inline-flex items-center justify-center gap-2 rounded-md text-sm font-medium disabled:cursor-not-allowed disabled:opacity-50',
                variant === 'primary' && 'bg-emerald-600 px-4 py-2 text-white hover:bg-emerald-700',
                variant === 'secondary' &&
                    'border border-neutral-300 px-4 py-2 hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800',
                variant === 'link' && 'text-emerald-700 hover:underline dark:text-emerald-400',
                className,
            )}
            {...rest}
        >
            {busy && <Loader2 className="size-4 animate-spin" aria-hidden/>}
            {children}
        </button>
    )
}

export function Field({label, children}: {label: string; children: ReactNode}) {
    return (
        <label className="mb-3 block">
            <span className="mb-1 block text-sm font-medium">{label}</span>
            {children}
        </label>
    )
}

export function TextInput({className, ...rest}: InputHTMLAttributes<HTMLInputElement>) {
    return <input autoComplete="off" spellCheck={false} className={clsx(inputClass, className)} {...rest}/>
}

/** Password input with a show/hide toggle. Never autofilled or spell-checked. */
export function PasswordInput({className, ...rest}: Omit<InputHTMLAttributes<HTMLInputElement>, 'type'>) {
    const {t} = useTranslation()
    const [visible, setVisible] = useState(false)
    const Icon = visible ? EyeOff : Eye
    return (
        <div className="relative">
            <input
                type={visible ? 'text' : 'password'}
                autoComplete="off"
                spellCheck={false}
                className={clsx(inputClass, 'pr-10', className)}
                {...rest}
            />
            <button
                type="button"
                tabIndex={-1}
                onClick={() => setVisible((v) => !v)}
                title={visible ? t('actions.hide') : t('actions.show')}
                aria-label={visible ? t('actions.hide') : t('actions.show')}
                className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
            >
                <Icon className="size-4" aria-hidden/>
            </button>
        </div>
    )
}

export function Alert({children, tone = 'error'}: {children: ReactNode; tone?: 'error' | 'warning'}) {
    return (
        <div
            role="alert"
            className={clsx(
                'mb-3 flex gap-2 rounded-md px-3 py-2 text-sm',
                tone === 'error' && 'bg-red-50 text-red-800 dark:bg-red-950/50 dark:text-red-300',
                tone === 'warning' && 'bg-amber-50 text-amber-900 dark:bg-amber-950/50 dark:text-amber-300',
            )}
        >
            <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden/>
            <div>{children}</div>
        </div>
    )
}
