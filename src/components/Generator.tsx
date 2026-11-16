import {useCallback, useEffect, useState} from 'react'
import {Copy, RefreshCw} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {api, type Generated, type GeneratorOptions} from '../lib/api'
import {copyText} from '../lib/copy'
import {errorMessage} from '../lib/errors'
import {readPref, writePref} from '../lib/prefs'
import {Alert, Button} from './ui'

const DEFAULT_CHARS: Extract<GeneratorOptions, {mode: 'chars'}> = {
    mode: 'chars',
    length: 20,
    lower: true,
    upper: true,
    digits: true,
    symbols: true,
    excludeAmbiguous: false,
}
const DEFAULT_PASSPHRASE: Extract<GeneratorOptions, {mode: 'passphrase'}> = {
    mode: 'passphrase',
    words: 5,
    separator: '-',
    capitalize: true,
    addDigit: true,
}

/** Generator options (never the generated value) are remembered per device. */
function loadOptions(): {chars: typeof DEFAULT_CHARS; passphrase: typeof DEFAULT_PASSPHRASE; mode: 'chars' | 'passphrase'} {
    try {
        const saved = JSON.parse(readPref('generator') ?? '{}')
        return {
            chars: {...DEFAULT_CHARS, ...saved.chars, mode: 'chars'},
            passphrase: {...DEFAULT_PASSPHRASE, ...saved.passphrase, mode: 'passphrase'},
            mode: saved.mode === 'passphrase' ? 'passphrase' : 'chars',
        }
    } catch {
        return {chars: DEFAULT_CHARS, passphrase: DEFAULT_PASSPHRASE, mode: 'chars'}
    }
}

/**
 * Password generator (runs in Rust with OsRng). `onUse` adds a "Use" button, e.g.
 * to fill the entry form.
 */
export function Generator({onUse}: {onUse?: (password: string) => void}) {
    const {t} = useTranslation()
    const [state, setState] = useState(loadOptions)
    const [result, setResult] = useState<Generated | null>(null)
    const [error, setError] = useState<string | null>(null)
    const options: GeneratorOptions = state.mode === 'chars' ? state.chars : state.passphrase

    const optionsKey = JSON.stringify(options)

    const regenerate = useCallback(() => {
        api.generatePassword(JSON.parse(optionsKey) as GeneratorOptions)
            .then((generated) => {
                setError(null)
                setResult(generated)
            })
            .catch((e) => {
                setResult(null)
                setError(errorMessage(t, e))
            })
    }, [optionsKey, t])

    useEffect(() => {
        regenerate()
        writePref('generator', JSON.stringify({mode: state.mode, chars: state.chars, passphrase: state.passphrase}))
    }, [regenerate, state])

    const setChars = (patch: Partial<typeof DEFAULT_CHARS>) => setState((s) => ({...s, chars: {...s.chars, ...patch}}))
    const setPhrase = (patch: Partial<typeof DEFAULT_PASSPHRASE>) =>
        setState((s) => ({...s, passphrase: {...s.passphrase, ...patch}}))
    const c = state.chars
    const p = state.passphrase
    const noSet = !c.lower && !c.upper && !c.digits && !c.symbols

    return (
        <div>
            <div role="tablist" className="mb-4 flex rounded-md border border-neutral-200 p-0.5 text-sm dark:border-neutral-700">
                {(['chars', 'passphrase'] as const).map((mode) => (
                    <button
                        key={mode}
                        type="button"
                        role="tab"
                        aria-selected={state.mode === mode}
                        onClick={() => setState((s) => ({...s, mode}))}
                        className={clsx(
                            'flex-1 rounded px-3 py-1.5',
                            state.mode === mode ? 'bg-neutral-200 font-medium dark:bg-neutral-700' : 'text-neutral-500',
                        )}
                    >
                        {t(`generator.${mode}`)}
                    </button>
                ))}
            </div>

            <div className="mb-3 flex items-center gap-2 rounded-lg bg-neutral-100 p-3 dark:bg-neutral-800">
                <code data-selectable className="min-h-6 flex-1 font-mono text-base break-all">
                    {result?.password ?? ''}
                </code>
                <button
                    type="button"
                    title={t('generator.regenerate')}
                    onClick={regenerate}
                    className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
                >
                    <RefreshCw className="size-4" aria-hidden/>
                </button>
                <button
                    type="button"
                    title={t('actions.copy')}
                    disabled={!result}
                    onClick={() => result && void copyText(t, result.password)}
                    className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 disabled:opacity-40 dark:hover:bg-neutral-700"
                >
                    <Copy className="size-4" aria-hidden/>
                </button>
            </div>
            <p className="mb-4 text-xs text-neutral-500">
                {result && t('generator.entropy', {bits: Math.round(result.entropyBits)})}
            </p>
            {error && <Alert>{noSet ? t('generator.noSet') : error}</Alert>}

            {state.mode === 'chars' ? (
                <div className="space-y-2 text-sm">
                    <label className="flex items-center gap-3">
                        <span className="w-24">{t('generator.length')}</span>
                        <input
                            type="range"
                            min={8}
                            max={128}
                            value={c.length}
                            onChange={(e) => setChars({length: Number(e.target.value)})}
                            className="flex-1 accent-emerald-600"
                        />
                        <span className="w-8 text-right tabular-nums">{c.length}</span>
                    </label>
                    <div className="grid grid-cols-2 gap-2">
                        <Check label="a–z" checked={c.lower} onChange={(v) => setChars({lower: v})}/>
                        <Check label="A–Z" checked={c.upper} onChange={(v) => setChars({upper: v})}/>
                        <Check label="0–9" checked={c.digits} onChange={(v) => setChars({digits: v})}/>
                        <Check label="!@#$…" checked={c.symbols} onChange={(v) => setChars({symbols: v})}/>
                    </div>
                    <Check
                        label={t('generator.excludeAmbiguous')}
                        checked={c.excludeAmbiguous}
                        onChange={(v) => setChars({excludeAmbiguous: v})}
                    />
                </div>
            ) : (
                <div className="space-y-2 text-sm">
                    <label className="flex items-center gap-3">
                        <span className="w-24">{t('generator.words')}</span>
                        <input
                            type="range"
                            min={3}
                            max={20}
                            value={p.words}
                            onChange={(e) => setPhrase({words: Number(e.target.value)})}
                            className="flex-1 accent-emerald-600"
                        />
                        <span className="w-8 text-right tabular-nums">{p.words}</span>
                    </label>
                    <label className="flex items-center gap-3">
                        <span className="w-24">{t('generator.separator')}</span>
                        <input
                            value={p.separator}
                            maxLength={3}
                            onChange={(e) => setPhrase({separator: e.target.value})}
                            className="w-16 rounded border border-neutral-300 bg-white px-2 py-1 font-mono dark:border-neutral-700 dark:bg-neutral-900"
                        />
                    </label>
                    <Check label={t('generator.capitalize')} checked={p.capitalize} onChange={(v) => setPhrase({capitalize: v})}/>
                    <Check label={t('generator.addDigit')} checked={p.addDigit} onChange={(v) => setPhrase({addDigit: v})}/>
                </div>
            )}

            {onUse && (
                <div className="mt-5 flex justify-end">
                    <Button disabled={!result} onClick={() => result && onUse(result.password)}>
                        {t('generator.use')}
                    </Button>
                </div>
            )}
        </div>
    )
}

function Check({label, checked, onChange}: {label: string; checked: boolean; onChange: (v: boolean) => void}) {
    return (
        <label className="flex items-center gap-2">
            <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="accent-emerald-600"/>
            {label}
        </label>
    )
}
