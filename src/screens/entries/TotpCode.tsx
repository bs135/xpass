import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {api} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {formatCode, secondsLeft} from '../../lib/totp'

/**
 * Current code of a TOTP field (v0.8 plan V4). Rust computes it from the secret,
 * which never reaches the WebView; the code is fetched again when it expires, only
 * while this view is mounted (entry open, vault unlocked).
 */
export function TotpCode({entryId, index, version}: {entryId: string; index: number; version: string}) {
    const {t} = useTranslation()
    const [state, setState] = useState<{code: string; period: number; expiresAt: number} | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [now, setNow] = useState(() => Date.now())

    useEffect(() => {
        let current = true
        let timer = 0
        const load = () => {
            api.totpCode(entryId, index)
                .then((c) => {
                    if (!current) return
                    setError(null)
                    setState({code: c.code, period: c.period, expiresAt: Date.now() + c.remainingMs})
                    // A little after the boundary, so the next call gets the new code.
                    timer = window.setTimeout(load, c.remainingMs + 100)
                })
                .catch((e) => {
                    if (current) setError(errorMessage(t, e))
                })
        }
        load()
        return () => {
            current = false
            window.clearTimeout(timer)
        }
    }, [entryId, index, version, t])

    useEffect(() => {
        const id = window.setInterval(() => setNow(Date.now()), 250)
        return () => window.clearInterval(id)
    }, [])

    if (error) return <span className="text-sm text-red-700 dark:text-red-400">{error}</span>
    if (!state) return <span className="text-neutral-400">…</span>
    const left = secondsLeft(state.expiresAt, now)
    return (
        <span className="flex items-center gap-3">
            <span data-selectable className="font-mono text-lg tracking-wider" aria-label={t('totp.code')}>
                {formatCode(state.code)}
            </span>
            <span
                className="flex items-center gap-1 text-xs text-neutral-500"
                title={t('totp.expires', {seconds: left})}
            >
                <svg viewBox="0 0 20 20" className="size-4 -rotate-90" aria-hidden>
                    <circle cx="10" cy="10" r="8" fill="none" strokeWidth="3" className="stroke-neutral-200 dark:stroke-neutral-700"/>
                    <circle
                        cx="10"
                        cy="10"
                        r="8"
                        fill="none"
                        strokeWidth="3"
                        pathLength={state.period}
                        strokeDasharray={`${Math.min(left, state.period)} ${state.period}`}
                        className={left <= 5 ? 'stroke-amber-500' : 'stroke-emerald-500'}
                    />
                </svg>
                {left}s
            </span>
        </span>
    )
}
