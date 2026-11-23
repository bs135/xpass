import {useMemo, useRef, useState} from 'react'
import {AlertTriangle, FileUp, Trash2} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Modal} from '../../components/Modal'
import {Alert, Button, Field} from '../../components/ui'
import {
    api,
    type DuplicateChoice,
    IMPORT_PRESETS,
    type ImportMapping,
    type ImportPreset,
    type ImportPreview,
    type ImportPreviewRow,
    type ImportSummary,
    isAppError,
} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useEntries} from '../../stores/entries'
import {useToast} from '../../stores/toast'

const selectClass =
    'w-full rounded-md border border-neutral-300 bg-white px-3 py-2 dark:border-neutral-700 dark:bg-neutral-900'

/** Mapping targets shown in the generic mapping table, in this order. */
const MAPPING_FIELDS = ['title', 'username', 'password', 'url', 'notes', 'tags', 'favorite'] as const

/** Rows shown before "Show all": keeps a 10 000-row file responsive. */
const ROWS_SHOWN = 200

/**
 * Settings → Import & export → Import from CSV. The file is picked in a native dialog
 * and parsed in Rust; this UI only sees a preview without passwords or notes.
 */
export function ImportSettings() {
    const {t} = useTranslation()
    const [preview, setPreview] = useState<ImportPreview | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const open = async () => {
        setError(null)
        setBusy(true)
        try {
            const result = await api.importOpen(null)
            if (result) setPreview(result)
        } catch (e) {
            setError(importError(t, e))
        } finally {
            setBusy(false)
        }
    }

    return (
        <>
            <h3 className="mb-1 font-medium">{t('import.title')}</h3>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('import.intro')}</p>
            {error && <Alert>{error}</Alert>}
            <Button variant="secondary" busy={busy} onClick={() => void open()}>
                <FileUp className="size-4" aria-hidden/>
                {t('import.open')}
            </Button>
            {preview && <ImportWizard initial={preview} onClose={() => setPreview(null)}/>}
        </>
    )
}

/** Error text for import commands; file errors are about the CSV, not the vault. */
function importError(t: ReturnType<typeof useTranslation>['t'], e: unknown): string {
    if (isAppError(e) && e.code === 'io') return t('import.readFailed', {kind: e.kind})
    return errorMessage(t, e)
}

function ImportWizard({initial, onClose}: {initial: ImportPreview; onClose: () => void}) {
    const {t} = useTranslation()
    const [preview, setPreview] = useState(initial)
    const [excluded, setExcluded] = useState<Set<number>>(new Set())
    const [duplicates, setDuplicates] = useState<DuplicateChoice>('skip')
    const [summary, setSummary] = useState<ImportSummary | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [expired, setExpired] = useState(false)
    const committed = useRef(false)

    // Closing without importing drops the parsed rows in Rust. Done here, not in an
    // effect cleanup: StrictMode runs cleanups right after the first mount in dev,
    // which cancelled the import as soon as the wizard opened. Unmounting on lock
    // needs nothing: Rust drops the pending import itself.
    const cancel = () => {
        if (!committed.current) void api.importCancel().catch(() => undefined)
        onClose()
    }

    const fail = (e: unknown) => {
        if (isAppError(e) && e.code === 'import_expired') setExpired(true)
        setError(importError(t, e))
    }

    const remap = async (preset: ImportPreset, mapping: ImportMapping | null) => {
        setError(null)
        setBusy(true)
        try {
            setPreview(await api.importSetMapping(preset, mapping))
            setExcluded(new Set())
        } catch (e) {
            fail(e)
        } finally {
            setBusy(false)
        }
    }

    const commit = async () => {
        setError(null)
        setBusy(true)
        try {
            const result = await api.importCommit(duplicates, [...excluded])
            committed.current = true
            setSummary(result)
            await useEntries.getState().refresh().catch(() => undefined)
        } catch (e) {
            fail(e)
        } finally {
            setBusy(false)
        }
    }

    const toImport = useMemo(
        () =>
            preview.rows.filter(
                (r) =>
                    !excluded.has(r.index) &&
                    r.duplicate !== 'exact' &&
                    !(r.duplicate === 'possible' && duplicates === 'skip'),
            ).length,
        [preview, excluded, duplicates],
    )

    if (summary) return <ImportResult summary={summary} onClose={onClose}/>

    return (
        <Modal title={t('import.wizardTitle')} wide onClose={cancel}>
            <p className="mb-3 truncate text-sm text-neutral-600 dark:text-neutral-400">
                {t('import.file', {name: preview.fileName})}
            </p>
            {error && <Alert>{error}</Alert>}
            {expired ? (
                <div className="flex justify-end">
                    <Button onClick={cancel}>{t('actions.close')}</Button>
                </div>
            ) : (
                <>
                    <div className="grid gap-x-4 sm:grid-cols-2">
                        <Field label={t('import.preset')}>
                            <select
                                className={selectClass}
                                value={preview.preset}
                                disabled={busy}
                                onChange={(e) => void remap(e.target.value as ImportPreset, null)}
                            >
                                {IMPORT_PRESETS.map((p) => (
                                    <option key={p} value={p}>
                                        {t(`import.presets.${p}`)}
                                        {p === preview.detected ? ` ${t('import.detected')}` : ''}
                                    </option>
                                ))}
                            </select>
                        </Field>
                    </div>

                    {preview.preset === 'generic' && preview.mapping && (
                        <MappingTable
                            headers={preview.headers}
                            mapping={preview.mapping}
                            disabled={busy}
                            onChange={(m) => void remap('generic', m)}
                        />
                    )}

                    <Counts preview={preview}/>

                    {preview.counts.possible > 0 && (
                        <fieldset className="mb-3">
                            <legend className="mb-1 text-sm font-medium">
                                {t('import.possibleQuestion', {count: preview.counts.possible})}
                            </legend>
                            {(['skip', 'new', 'update'] as const).map((choice) => (
                                <label key={choice} className="flex items-center gap-2 text-sm">
                                    <input
                                        type="radio"
                                        name="duplicates"
                                        className="accent-emerald-600"
                                        checked={duplicates === choice}
                                        onChange={() => setDuplicates(choice)}
                                    />
                                    {t(`import.duplicates.${choice}`)}
                                </label>
                            ))}
                        </fieldset>
                    )}

                    <RowList
                        rows={preview.rows}
                        excluded={excluded}
                        onToggle={(index) =>
                            setExcluded((old) => {
                                const next = new Set(old)
                                if (next.has(index)) next.delete(index)
                                else next.add(index)
                                return next
                            })
                        }
                    />

                    <div className="mt-4 flex justify-end gap-2">
                        <Button variant="secondary" onClick={cancel}>
                            {t('actions.cancel')}
                        </Button>
                        <Button busy={busy} disabled={toImport === 0} onClick={() => void commit()}>
                            {t('import.commit', {count: toImport})}
                        </Button>
                    </div>
                </>
            )}
        </Modal>
    )
}

function Counts({preview}: {preview: ImportPreview}) {
    const {t} = useTranslation()
    const {counts, skipped} = preview
    return (
        <p className="mb-3 text-sm">
            {t('import.countNew', {count: counts.new})}
            {' · '}
            {t('import.countExact', {count: counts.exact})}
            {' · '}
            {t('import.countPossible', {count: counts.possible})}
            {skipped.length > 0 && (
                <>
                    {' · '}
                    <span className="text-neutral-500" title={skippedLines(skipped)}>
                        {t('import.countSkipped', {count: skipped.length})}
                    </span>
                </>
            )}
        </p>
    )
}

function skippedLines(skipped: ImportPreview['skipped']): string {
    return skipped.map((s) => s.line).join(', ')
}

function MappingTable({
    headers,
    mapping,
    disabled,
    onChange,
}: {
    headers: string[]
    mapping: ImportMapping
    disabled: boolean
    onChange: (m: ImportMapping) => void
}) {
    const {t} = useTranslation()
    const column = (i: number) => headers[i] || t('import.column', {n: i + 1})
    return (
        <div className="mb-3 rounded-md border border-neutral-200 p-3 dark:border-neutral-800">
            <p className="mb-2 text-sm text-neutral-600 dark:text-neutral-400">{t('import.mappingIntro')}</p>
            <div className="grid gap-x-4 sm:grid-cols-2">
                {MAPPING_FIELDS.map((field) => (
                    <Field key={field} label={t(`import.fields.${field}`)}>
                        <select
                            className={selectClass}
                            disabled={disabled}
                            value={mapping[field] ?? ''}
                            onChange={(e) =>
                                onChange({...mapping, [field]: e.target.value === '' ? null : Number(e.target.value)})
                            }
                        >
                            <option value="">{t('import.notMapped')}</option>
                            {headers.map((_, i) => (
                                <option key={i} value={i}>
                                    {column(i)}
                                </option>
                            ))}
                        </select>
                    </Field>
                ))}
                <Field label={t('import.unmapped')}>
                    <select
                        className={selectClass}
                        disabled={disabled}
                        value={mapping.unmapped}
                        onChange={(e) => onChange({...mapping, unmapped: e.target.value as ImportMapping['unmapped']})}
                    >
                        {(['ignore', 'notes', 'fields'] as const).map((u) => (
                            <option key={u} value={u}>
                                {t(`import.unmappedChoice.${u}`)}
                            </option>
                        ))}
                    </select>
                </Field>
            </div>
        </div>
    )
}

function RowList({
    rows,
    excluded,
    onToggle,
}: {
    rows: ImportPreviewRow[]
    excluded: Set<number>
    onToggle: (index: number) => void
}) {
    const {t} = useTranslation()
    const [all, setAll] = useState(false)
    const shown = all ? rows : rows.slice(0, ROWS_SHOWN)
    return (
        <div className="max-h-72 overflow-auto rounded-md border border-neutral-200 dark:border-neutral-800">
            <table className="w-full text-left text-sm">
                <thead className="sticky top-0 bg-neutral-50 text-xs text-neutral-500 dark:bg-neutral-900">
                    <tr>
                        <th className="w-8 px-2 py-1">
                            <span className="sr-only">{t('import.include')}</span>
                        </th>
                        <th className="px-2 py-1">{t('import.fields.title')}</th>
                        <th className="px-2 py-1">{t('import.fields.username')}</th>
                        <th className="px-2 py-1">{t('import.site')}</th>
                        <th className="px-2 py-1">
                            <span className="sr-only">{t('import.status')}</span>
                        </th>
                    </tr>
                </thead>
                <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
                    {shown.map((r) => {
                        const exact = r.duplicate === 'exact'
                        return (
                            <tr key={r.index} className={clsx((exact || excluded.has(r.index)) && 'text-neutral-400')}>
                                <td className="px-2 py-1">
                                    <input
                                        type="checkbox"
                                        className="accent-emerald-600"
                                        aria-label={t('import.include')}
                                        checked={!exact && !excluded.has(r.index)}
                                        disabled={exact}
                                        onChange={() => onToggle(r.index)}
                                    />
                                </td>
                                <td className="max-w-48 truncate px-2 py-1">
                                    {r.title}
                                    {r.warnings.length > 0 && (
                                        <span
                                            role="img"
                                            title={r.warnings.map((w) => t(`import.warnings.${w}`)).join(' ')}
                                            aria-label={r.warnings.map((w) => t(`import.warnings.${w}`)).join(' ')}
                                        >
                                            <AlertTriangle className="ml-1 inline size-3.5 text-amber-600" aria-hidden/>
                                        </span>
                                    )}
                                </td>
                                <td className="max-w-40 truncate px-2 py-1">{r.username}</td>
                                <td className="max-w-40 truncate px-2 py-1">{r.urlHost ?? ''}</td>
                                <td className="px-2 py-1 text-right">
                                    {r.duplicate !== 'none' && (
                                        <span
                                            className={clsx(
                                                'rounded px-1.5 py-0.5 text-xs whitespace-nowrap',
                                                exact
                                                    ? 'bg-neutral-100 dark:bg-neutral-800'
                                                    : 'bg-amber-100 text-amber-900 dark:bg-amber-950 dark:text-amber-300',
                                            )}
                                        >
                                            {t(`import.badge.${r.duplicate}`)}
                                        </span>
                                    )}
                                    {!r.hasPassword && (
                                        <span className="ml-1 text-xs whitespace-nowrap text-neutral-500">
                                            {t('import.noPassword')}
                                        </span>
                                    )}
                                </td>
                            </tr>
                        )
                    })}
                </tbody>
            </table>
            {!all && rows.length > ROWS_SHOWN && (
                <div className="p-2 text-center">
                    <Button variant="link" onClick={() => setAll(true)}>
                        {t('import.showAll', {count: rows.length})}
                    </Button>
                </div>
            )}
        </div>
    )
}

function ImportResult({summary, onClose}: {summary: ImportSummary; onClose: () => void}) {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    const [confirm, setConfirm] = useState(false)
    const [deleted, setDeleted] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const remove = async () => {
        setConfirm(false)
        setError(null)
        try {
            await api.importDeleteSource()
            setDeleted(true)
            show(t('import.deleted'))
        } catch (e) {
            setError(importError(t, e))
        }
    }

    // Shown instead of the result (two stacked dialogs would both close on Esc).
    if (confirm) {
        return (
            <ConfirmDialog
                title={t('import.deleteTitle')}
                message={t('import.deleteMessage')}
                confirmLabel={t('import.deleteConfirm')}
                onConfirm={() => void remove()}
                onCancel={() => setConfirm(false)}
            />
        )
    }

    return (
        <Modal title={t('import.doneTitle')} onClose={onClose}>
            <p className="mb-3 text-sm">
                {t('import.added', {count: summary.added})}
                {' · '}
                {t('import.updated', {count: summary.updated})}
                {' · '}
                {t('import.skipped', {count: summary.skipped})}
            </p>
            {summary.added + summary.updated > 0 && (
                <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('import.undoHint')}</p>
            )}
            {error && <Alert>{error}</Alert>}
            <Alert tone="warning">{t('import.deleteHint')}</Alert>
            <div className="flex flex-wrap justify-end gap-2">
                {!deleted && (
                    <Button variant="secondary" onClick={() => setConfirm(true)}>
                        <Trash2 className="size-4" aria-hidden/>
                        {t('import.deleteSource')}
                    </Button>
                )}
                <Button onClick={onClose}>{t('actions.close')}</Button>
            </div>
        </Modal>
    )
}
