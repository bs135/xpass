import {type FormEvent, useMemo, useState} from 'react'
import {Printer, ShieldAlert} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Alert, Button, Card, Field, TextInput} from '../components/ui'
import {normalizeGroup, pickConfirmGroups, splitGroups} from '../lib/recoveryKey'
import {useSession} from '../stores/session'

/**
 * Shows the recovery key once, offers printing (print-only sheet below), then asks
 * the user to re-type two random groups before continuing.
 */
export function EmergencyKitScreen() {
    const {t, i18n} = useTranslation()
    const recoveryKey = useSession((s) => s.recoveryKey)
    const finishKit = useSession((s) => s.finishKit)
    const groups = useMemo(() => splitGroups(recoveryKey ?? ''), [recoveryKey])
    const [step, setStep] = useState<'show' | 'confirm'>('show')
    const [createdAt] = useState(() => new Date())
    const created = createdAt.toLocaleDateString(i18n.language)

    if (!recoveryKey) return null

    return (
        <>
            <div className="contents print:hidden">
                {step === 'show' ? (
                    <Card title={t('kit.title')}>
                        <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('kit.intro')}</p>
                        <KeyGrid groups={groups}/>
                        <Alert tone="warning">{t('kit.warning')}</Alert>
                        <div className="flex flex-wrap gap-2">
                            <Button variant="secondary" onClick={() => window.print()}>
                                <Printer className="size-4" aria-hidden/>
                                {t('kit.print')}
                            </Button>
                            <Button className="flex-1" onClick={() => setStep('confirm')}>
                                {t('kit.saved')}
                            </Button>
                        </div>
                    </Card>
                ) : (
                    <ConfirmStep groups={groups} onBack={() => setStep('show')} onDone={finishKit}/>
                )}
            </div>
            <PrintSheet groups={groups} created={created}/>
        </>
    )
}

function KeyGrid({groups}: {groups: string[]}) {
    return (
        <ol data-selectable className="mb-4 grid grid-cols-3 gap-2 rounded-lg bg-neutral-100 p-3 font-mono text-base dark:bg-neutral-800">
            {groups.map((g, i) => (
                <li key={i} className="flex items-baseline gap-1.5">
                    <span className="w-5 text-right text-xs text-neutral-400">{i + 1}</span>
                    <span className="tracking-wider">{g}</span>
                </li>
            ))}
        </ol>
    )
}

function ConfirmStep({groups, onBack, onDone}: {groups: string[]; onBack: () => void; onDone: () => void}) {
    const {t} = useTranslation()
    const asked = useMemo(() => pickConfirmGroups(groups.length), [groups])
    const [values, setValues] = useState<string[]>(() => asked.map(() => ''))
    const [error, setError] = useState(false)

    const submit = (e: FormEvent) => {
        e.preventDefault()
        const ok = asked.every((g, i) => normalizeGroup(values[i]) === groups[g])
        if (ok) {
            onDone()
        } else {
            setError(true)
        }
    }

    return (
        <Card title={t('kit.confirmTitle')}>
            <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">
                {t('kit.confirmIntro', {a: asked[0] + 1, b: asked[1] + 1})}
            </p>
            <form onSubmit={submit}>
                <div className="flex gap-3">
                    {asked.map((g, i) => (
                        <Field key={g} label={t('kit.group', {n: g + 1})}>
                            <TextInput
                                autoFocus={i === 0}
                                maxLength={7}
                                className="font-mono uppercase tracking-wider"
                                value={values[i]}
                                onChange={(e) => {
                                    setError(false)
                                    setValues((v) => v.map((old, j) => (j === i ? e.target.value : old)))
                                }}
                            />
                        </Field>
                    ))}
                </div>
                {error && <Alert>{t('kit.confirmMismatch')}</Alert>}
                <div className="flex gap-2">
                    <Button variant="secondary" onClick={onBack}>
                        {t('kit.back')}
                    </Button>
                    <Button type="submit" className="flex-1" disabled={values.some((v) => !v.trim())}>
                        {t('kit.done')}
                    </Button>
                </div>
            </form>
        </Card>
    )
}

/** Only visible when printing (or saving as PDF). */
function PrintSheet({groups, created}: {groups: string[]; created: string}) {
    const {t} = useTranslation()
    const steps = t('kit.printSteps', {returnObjects: true}) as string[]
    return (
        <section className="hidden bg-white p-10 text-black print:block">
            <div className="mb-6 flex items-center gap-3 border-b border-black pb-4">
                <ShieldAlert className="size-8" aria-hidden/>
                <div>
                    <h1 className="text-2xl font-bold">{t('kit.printTitle')}</h1>
                    <p className="text-sm">{t('kit.printCreated', {date: created})}</p>
                </div>
            </div>
            <h2 className="mb-2 font-semibold">{t('kit.printKeyLabel')}</h2>
            <p className="mb-6 rounded border border-black p-4 font-mono text-xl tracking-wider">{groups.join('-')}</p>
            <h2 className="mb-2 font-semibold">{t('kit.printHowTo')}</h2>
            <ol className="mb-6 list-decimal pl-6 text-sm">
                {steps.map((s, i) => (
                    <li key={i}>{s}</li>
                ))}
            </ol>
            <p className="text-sm font-semibold">{t('kit.printKeep')}</p>
        </section>
    )
}
