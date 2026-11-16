import {useEffect, useState} from 'react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {api, type Strength} from '../lib/api'

const COLORS = ['bg-red-500', 'bg-orange-500', 'bg-amber-500', 'bg-emerald-500', 'bg-emerald-600']

/**
 * Live zxcvbn estimate (computed in Rust), debounced. Reports each score together
 * with the password it belongs to, so the parent never uses a stale score.
 */
export function StrengthMeter({
    password,
    onScore,
}: {
    password: string
    onScore: (password: string, score: number) => void
}) {
    const {t, i18n} = useTranslation()
    const [result, setResult] = useState<{password: string; strength: Strength} | null>(null)

    useEffect(() => {
        if (!password) return
        let cancelled = false
        const id = window.setTimeout(() => {
            api.passwordStrength(password)
                .then((strength) => {
                    if (cancelled) return
                    setResult({password, strength})
                    onScore(password, strength.score)
                })
                .catch((e) => console.error('password_strength failed', e))
        }, 150)
        return () => {
            cancelled = true
            window.clearTimeout(id)
        }
    }, [password, onScore])

    const strength = password && result?.password === password ? result.strength : null
    const score = strength?.score ?? 0
    // zxcvbn feedback is English only; other languages get a generic hint.
    const feedback = i18n.language === 'en' ? (strength?.warning ?? strength?.suggestions[0]) : null

    return (
        <div className="mb-3" aria-live="polite">
            <div className="mb-1 flex gap-1">
                {[0, 1, 2, 3].map((i) => (
                    <div
                        key={i}
                        className={clsx(
                            'h-1.5 flex-1 rounded-full',
                            strength && i < Math.max(score, 1) ? COLORS[score] : 'bg-neutral-200 dark:bg-neutral-700',
                        )}
                    />
                ))}
            </div>
            <div className="flex justify-between gap-3 text-xs text-neutral-500 dark:text-neutral-400">
                <span>{strength ? t(`strength.level${score}`) : t('strength.hint')}</span>
                {feedback && <span className="text-right">{feedback}</span>}
            </div>
        </div>
    )
}
