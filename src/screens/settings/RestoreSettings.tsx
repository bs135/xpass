import {useCallback, useEffect, useState} from 'react'
import {History} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {ConfirmDialog} from '../../components/ConfirmDialog'
import {Alert, Button} from '../../components/ui'
import {api, type BackupInfo} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {useEntries} from '../../stores/entries'
import {useSync} from '../../stores/sync'
import {useToast} from '../../stores/toast'
import {PasswordPrompt} from './PasswordPrompt'

/**
 * Settings → Restore a previous version: the local backups (one per save, the
 * newest ten). A restore is saved as a new change, so it also wins on the other
 * devices after the next sync.
 */
export function RestoreSettings() {
    const {t, i18n} = useTranslation()
    const show = useToast((s) => s.show)
    const syncOn = useSync((s) => s.status.phase !== 'off')
    const [backups, setBackups] = useState<BackupInfo[] | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [selected, setSelected] = useState<BackupInfo | null>(null)

    const load = useCallback(() => {
        api.listBackups()
            .then(setBackups)
            .catch((e) => setError(errorMessage(t, e)))
    }, [t])

    useEffect(load, [load])

    const when = (b: BackupInfo) =>
        b.savedAt === null
            ? b.name
            : new Date(b.savedAt).toLocaleString(i18n.language, {dateStyle: 'medium', timeStyle: 'medium'})

    const restore = async (b: BackupInfo, password: string | null) => {
        await api.restoreBackup(b.name, password)
        setSelected(null)
        await useEntries.getState().refresh().catch(() => undefined)
        load()
        show(t('restore.done'))
    }

    const explain = (b: BackupInfo) =>
        [t('restore.explain', {date: when(b)}), t('restore.keepCurrent'), syncOn ? t('restore.syncs') : '']
            .filter(Boolean)
            .join(' ')

    return (
        <>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('restore.intro')}</p>
            {error && <Alert>{error}</Alert>}
            {backups === null ? (
                !error && <p className="text-sm text-neutral-500">…</p>
            ) : backups.length === 0 ? (
                <p className="text-sm text-neutral-600 dark:text-neutral-400">{t('restore.none')}</p>
            ) : (
                <ul className="divide-y divide-neutral-200 rounded-md border border-neutral-200 text-sm dark:divide-neutral-800 dark:border-neutral-800">
                    {backups.map((b) => (
                        <li key={b.name} className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
                            <div className="min-w-0">
                                <p className="font-medium">{when(b)}</p>
                                <p className="text-xs text-neutral-500">
                                    {b.kind === 'current'
                                        ? t('restore.entries', {count: b.entries ?? 0})
                                        : t(`restore.kind.${b.kind}`)}
                                    {' · '}
                                    {t('restore.size', {kb: Math.max(1, Math.round(b.size / 1024))})}
                                </p>
                            </div>
                            <Button
                                variant="secondary"
                                disabled={b.kind === 'otherVault' || b.kind === 'unreadable'}
                                onClick={() => {
                                    setError(null)
                                    setSelected(b)
                                }}
                            >
                                <History className="size-4" aria-hidden/>
                                {t('restore.button')}
                            </Button>
                        </li>
                    ))}
                </ul>
            )}

            {selected?.kind === 'current' && (
                <ConfirmDialog
                    title={t('restore.confirmTitle')}
                    message={explain(selected)}
                    confirmLabel={t('restore.button')}
                    onConfirm={() =>
                        void restore(selected, null).catch((e) => {
                            setSelected(null)
                            setError(errorMessage(t, e))
                        })
                    }
                    onCancel={() => setSelected(null)}
                />
            )}
            {selected?.kind === 'olderKey' && (
                <PasswordPrompt
                    title={t('restore.confirmTitle')}
                    label={t('restore.oldPassword')}
                    confirmLabel={t('restore.button')}
                    onConfirm={(password) => restore(selected, password)}
                    onClose={() => setSelected(null)}
                >
                    <p className="mb-2 text-sm text-neutral-600 dark:text-neutral-400">{explain(selected)}</p>
                    <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('restore.olderKeyExplain')}</p>
                </PasswordPrompt>
            )}
        </>
    )
}
