import {useState} from 'react'
import {ArrowDown, ArrowUp, Eye, KeyRound, Plus, Trash2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {Button, PasswordInput, TextInput} from '../../components/ui'
import {api} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {type FieldRow, MAX_FIELDS, move, newRow} from '../../lib/fields'
import {useToast} from '../../stores/toast'
import {TotpDialog} from './TotpDialog'

/**
 * Custom fields of the entry form (v0.8 plan V6). A stored secret value is not
 * loaded: the row shows "stored value" until the user reveals it (same rules as the
 * entry view) or replaces it. Unchanged rows are saved as `keep`.
 */
export function FieldEditor({
    entryId,
    rows,
    onChange,
    issuer,
    account,
}: {
    entryId: string | null
    rows: FieldRow[]
    onChange: (rows: FieldRow[]) => void
    /** Defaults for a new one-time password: the entry's title and username. */
    issuer: string
    account: string
}) {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    const [addingTotp, setAddingTotp] = useState(false)

    const update = (i: number, patch: Partial<FieldRow>) => onChange(rows.map((r, j) => (j === i ? {...r, ...patch} : r)))

    const reveal = async (i: number) => {
        const row = rows[i]
        if (!entryId || !row.stored) return
        try {
            const value = await api.revealSecret(entryId, {kind: 'field', index: row.stored.index})
            update(i, {value, revealed: true})
        } catch (e) {
            show(errorMessage(t, e), 4000)
        }
    }

    return (
        <fieldset className="mb-4">
            <legend className="mb-2 text-sm font-medium">{t('fields.title')}</legend>
            {rows.length === 0 && <p className="mb-2 text-sm text-neutral-500">{t('fields.none')}</p>}
            <ul className="space-y-3">
                {rows.map((row, i) => {
                    const hidden = row.stored?.secret && !row.revealed && !row.edited
                    return (
                        <li
                            key={row.key}
                            className="rounded-md border border-neutral-200 p-3 dark:border-neutral-800"
                        >
                            <div className="mb-2 flex flex-wrap items-center gap-2">
                                <TextInput
                                    aria-label={t('fields.name')}
                                    placeholder={t('fields.name')}
                                    maxLength={100}
                                    className="min-w-40 flex-1"
                                    value={row.name}
                                    onChange={(e) => update(i, {name: e.target.value})}
                                />
                                <label className="flex items-center gap-1 text-sm">
                                    <input
                                        type="checkbox"
                                        checked={row.secret}
                                        onChange={(e) => update(i, {secret: e.target.checked})}
                                    />
                                    {t('fields.secret')}
                                </label>
                                <Button
                                    variant="secondary"
                                    title={t('fields.up')}
                                    aria-label={t('fields.up')}
                                    disabled={i === 0}
                                    onClick={() => onChange(move(rows, i, i - 1))}
                                >
                                    <ArrowUp className="size-4" aria-hidden/>
                                </Button>
                                <Button
                                    variant="secondary"
                                    title={t('fields.down')}
                                    aria-label={t('fields.down')}
                                    disabled={i === rows.length - 1}
                                    onClick={() => onChange(move(rows, i, i + 1))}
                                >
                                    <ArrowDown className="size-4" aria-hidden/>
                                </Button>
                                <Button
                                    variant="secondary"
                                    title={t('fields.remove')}
                                    aria-label={t('fields.remove')}
                                    onClick={() => onChange(rows.filter((_, j) => j !== i))}
                                >
                                    <Trash2 className="size-4" aria-hidden/>
                                </Button>
                            </div>
                            {hidden ? (
                                <div className="flex flex-wrap items-center gap-2 text-sm">
                                    <span className="flex-1 text-neutral-500">{t('fields.stored')}</span>
                                    <Button variant="secondary" onClick={() => void reveal(i)}>
                                        <Eye className="size-4" aria-hidden/>
                                        {t('fields.reveal')}
                                    </Button>
                                    <Button variant="secondary" onClick={() => update(i, {value: '', edited: true})}>
                                        {t('fields.replace')}
                                    </Button>
                                </div>
                            ) : row.secret ? (
                                <PasswordInput
                                    aria-label={t('fields.value')}
                                    placeholder={t('fields.value')}
                                    value={row.value}
                                    onChange={(e) => update(i, {value: e.target.value, edited: true})}
                                />
                            ) : (
                                <TextInput
                                    aria-label={t('fields.value')}
                                    placeholder={t('fields.value')}
                                    value={row.value}
                                    onChange={(e) => update(i, {value: e.target.value, edited: true})}
                                />
                            )}
                        </li>
                    )
                })}
            </ul>
            <div className="mt-3 flex flex-wrap gap-2">
                <Button
                    variant="secondary"
                    disabled={rows.length >= MAX_FIELDS}
                    onClick={() => onChange([...rows, newRow()])}
                >
                    <Plus className="size-4" aria-hidden/>
                    {t('fields.add')}
                </Button>
                <Button variant="secondary" disabled={rows.length >= MAX_FIELDS} onClick={() => setAddingTotp(true)}>
                    <KeyRound className="size-4" aria-hidden/>
                    {t('totp.addButton')}
                </Button>
            </div>
            {addingTotp && (
                <TotpDialog
                    issuer={issuer}
                    account={account}
                    onClose={() => setAddingTotp(false)}
                    onAdd={(uri) => {
                        setAddingTotp(false)
                        onChange([...rows, {...newRow(), name: 'TOTP', secret: true, value: uri}])
                    }}
                />
            )}
        </fieldset>
    )
}
