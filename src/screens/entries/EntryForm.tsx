import {type FormEvent, useState} from 'react'
import {Wand2} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Generator} from '../../components/Generator'
import {Modal} from '../../components/Modal'
import {Alert, Button, Field, PasswordInput, TextInput} from '../../components/ui'
import type {EntryDetail} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {type FieldRow, lostSecrets, rowsFromDetail, rowsProblem, toInput} from '../../lib/fields'
import {parseLines, parseTags} from '../../lib/search'
import {useEntries} from '../../stores/entries'
import {FieldEditor} from './FieldEditor'

const areaClass =
    'w-full rounded-md border border-neutral-300 bg-white px-3 py-2 outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/30 dark:border-neutral-700 dark:bg-neutral-900'

/**
 * Create (no `detail`) or edit form. The current password is not loaded: leaving
 * the field empty keeps it. A typed password lives only in this component's state.
 */
export function EntryForm({detail}: {detail: EntryDetail | null}) {
    const {t} = useTranslation()
    const {save, cancelEdit} = useEntries()
    const [title, setTitle] = useState(detail?.title ?? '')
    const [username, setUsername] = useState(detail?.username ?? '')
    const [password, setPassword] = useState('')
    const [urls, setUrls] = useState(detail?.urls.join('\n') ?? '')
    const [tags, setTags] = useState(detail?.tags.join(', ') ?? '')
    const [notes, setNotes] = useState(detail?.notes ?? '')
    const [favorite, setFavorite] = useState(detail?.favorite ?? false)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [generating, setGenerating] = useState(false)
    const [fields, setFields] = useState<FieldRow[]>(() => rowsFromDetail(detail?.fields ?? []))
    // Stored secret field values the save would drop (no history for fields, V7).
    const [confirmLost, setConfirmLost] = useState<string[] | null>(null)

    const submit = (e: FormEvent) => {
        e.preventDefault()
        if (!title.trim() || busy) return
        const problem = rowsProblem(fields)
        if (problem) {
            setError(t(`fields.problem.${problem}`))
            return
        }
        const lost = lostSecrets(fields, detail?.fields ?? [])
        if (lost.length > 0) {
            setConfirmLost(lost)
            return
        }
        void persist()
    }

    const persist = async () => {
        setConfirmLost(null)
        setBusy(true)
        setError(null)
        try {
            await save({
                id: detail?.id ?? null,
                title,
                username,
                // Editing: empty keeps the current password. Creating: empty means none.
                password: detail && password === '' ? null : password,
                urls: parseLines(urls),
                notes,
                tags: parseTags(tags),
                favorite,
                fields: toInput(fields),
            })
            setPassword('')
        } catch (err) {
            setError(errorMessage(t, err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <form
            onSubmit={submit}
            onKeyDown={(e) => {
                if (e.key === 'Escape') {
                    e.preventDefault()
                    cancelEdit()
                }
            }}
            className="mx-auto w-full max-w-2xl p-6"
        >
            <h1 className="mb-5 text-xl font-semibold">{t(detail ? 'entries.editTitle' : 'entries.newTitle')}</h1>
            <Field label={t('entries.title')}>
                <TextInput autoFocus required maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)}/>
            </Field>
            <Field label={t('entries.username')}>
                <TextInput value={username} onChange={(e) => setUsername(e.target.value)}/>
            </Field>
            <Field label={t('entries.password')}>
                <div className="flex gap-2">
                    <div className="flex-1">
                        <PasswordInput
                            value={password}
                            placeholder={detail?.hasPassword ? t('entries.passwordKeep') : ''}
                            onChange={(e) => setPassword(e.target.value)}
                        />
                    </div>
                    <Button variant="secondary" title={t('generator.title')} onClick={() => setGenerating(true)}>
                        <Wand2 className="size-4" aria-hidden/>
                        {t('generator.open')}
                    </Button>
                </div>
            </Field>
            <Field label={t('entries.urlsHint')}>
                <textarea rows={2} spellCheck={false} className={areaClass} value={urls} onChange={(e) => setUrls(e.target.value)}/>
            </Field>
            <Field label={t('entries.tagsHint')}>
                <TextInput value={tags} onChange={(e) => setTags(e.target.value)}/>
            </Field>
            <Field label={t('entries.notes')}>
                <textarea rows={5} className={areaClass} value={notes} onChange={(e) => setNotes(e.target.value)}/>
            </Field>
            <FieldEditor entryId={detail?.id ?? null} rows={fields} onChange={setFields}/>
            <label className="mb-5 flex items-center gap-2 text-sm">
                <input type="checkbox" checked={favorite} onChange={(e) => setFavorite(e.target.checked)}/>
                {t('entries.favorite')}
            </label>
            {error && <Alert>{error}</Alert>}
            <div className="flex justify-end gap-2">
                <Button variant="secondary" onClick={cancelEdit}>
                    {t('actions.cancel')}
                </Button>
                <Button type="submit" busy={busy} disabled={!title.trim()}>
                    {t('actions.save')}
                </Button>
            </div>
            {confirmLost && (
                <ConfirmDialog
                    title={t('fields.lostTitle')}
                    message={t('fields.lostMessage', {names: confirmLost.join(', ')})}
                    confirmLabel={t('actions.save')}
                    onConfirm={() => void persist()}
                    onCancel={() => setConfirmLost(null)}
                />
            )}
            {generating && (
                <Modal title={t('generator.title')} onClose={() => setGenerating(false)}>
                    <Generator
                        onUse={(generated) => {
                            setPassword(generated)
                            setGenerating(false)
                        }}
                    />
                </Modal>
            )}
        </form>
    )
}
