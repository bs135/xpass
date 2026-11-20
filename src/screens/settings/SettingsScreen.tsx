import {type ReactNode, useEffect, useState} from 'react'
import {ArrowLeft, ExternalLink} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {NewPasswordForm} from '../../components/NewPasswordForm'
import {Button, Field, PasswordInput} from '../../components/ui'
import {api, type AppInfo, AUTO_LOCK_CHOICES, CLIPBOARD_CHOICES} from '../../lib/api'
import {errorMessage} from '../../lib/errors'
import {THEMES, type Theme} from '../../lib/theme'
import {useSession} from '../../stores/session'
import {useSync} from '../../stores/sync'
import {useToast} from '../../stores/toast'
import {useUi} from '../../stores/ui'
import {HelloSettings} from './HelloSettings'
import {ImportSettings} from './ImportSettings'
import {PasswordPrompt} from './PasswordPrompt'
import {RestoreSettings} from './RestoreSettings'
import {SyncSettings} from './SyncSettings'

const selectClass =
    'w-full rounded-md border border-neutral-300 bg-white px-3 py-2 dark:border-neutral-700 dark:bg-neutral-900'

export function SettingsScreen() {
    const {t} = useTranslation()
    const go = useSession((s) => s.go)
    return (
        <div className="min-h-0 flex-1 overflow-auto">
            <div className="mx-auto w-full max-w-2xl p-6">
                <div className="mb-6 flex items-center gap-3">
                    <Button variant="secondary" onClick={() => go('unlocked')}>
                        <ArrowLeft className="size-4" aria-hidden/>
                        {t('actions.back')}
                    </Button>
                    <h1 className="text-xl font-semibold">{t('settings.title')}</h1>
                </div>
                <GeneralSection/>
                <SecuritySection/>
                <Section title={t('sync.title')}>
                    <SyncSettings/>
                </Section>
                <Section title={t('import.section')}>
                    <ImportSettings/>
                </Section>
                <Section title={t('restore.title')}>
                    <RestoreSettings/>
                </Section>
                <AboutSection/>
            </div>
        </div>
    )
}

function Section({title, children}: {title: string; children: ReactNode}) {
    return (
        <section className="mb-6 rounded-xl border border-neutral-200 p-5 dark:border-neutral-800">
            <h2 className="mb-4 text-lg font-semibold">{title}</h2>
            {children}
        </section>
    )
}

function GeneralSection() {
    const {t} = useTranslation()
    const {settings, theme, setTheme, setLanguage, update} = useUi()
    const show = useToast((s) => s.show)
    const save = (patch: Parameters<typeof update>[0]) =>
        update(patch)
            .then(() => show(t('settings.saved')))
            .catch((e) => show(errorMessage(t, e), 4000))

    return (
        <Section title={t('settings.general')}>
            <div className="grid gap-x-4 sm:grid-cols-2">
                <Field label={t('settings.theme')}>
                    <select className={selectClass} value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
                        {THEMES.map((v) => (
                            <option key={v} value={v}>
                                {t(`theme.${v}`)}
                            </option>
                        ))}
                    </select>
                </Field>
                <Field label={t('settings.language')}>
                    <select
                        className={selectClass}
                        value={settings.language ?? ''}
                        onChange={(e) => setLanguage(e.target.value === '' ? null : (e.target.value as 'en' | 'vi'))}
                    >
                        <option value="">{t('settings.languageSystem')}</option>
                        <option value="en">{t('language.en')}</option>
                        <option value="vi">{t('language.vi')}</option>
                    </select>
                </Field>
                <Field label={t('settings.autoLock')}>
                    <select
                        className={selectClass}
                        value={settings.autoLockMinutes ?? 'never'}
                        onChange={(e) =>
                            void save({autoLockMinutes: e.target.value === 'never' ? null : Number(e.target.value)})
                        }
                    >
                        {AUTO_LOCK_CHOICES.map((m) => (
                            <option key={m} value={m}>
                                {t('settings.minutes', {count: m})}
                            </option>
                        ))}
                        <option value="never">{t('settings.never')}</option>
                    </select>
                </Field>
                <Field label={t('settings.clipboard')}>
                    <select
                        className={selectClass}
                        value={settings.clipboardClearSeconds}
                        onChange={(e) => void save({clipboardClearSeconds: Number(e.target.value)})}
                    >
                        {CLIPBOARD_CHOICES.map((s) => (
                            <option key={s} value={s}>
                                {t('settings.seconds', {count: s})}
                            </option>
                        ))}
                    </select>
                </Field>
            </div>
            <p className="mb-4 text-xs text-neutral-500">{t('settings.autoLockHint')}</p>
            <label className="mb-2 flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    className="accent-emerald-600"
                    checked={settings.closeToTray}
                    onChange={(e) => void save({closeToTray: e.target.checked})}
                />
                {t('settings.closeToTray')}
            </label>
            <label className="flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    className="accent-emerald-600"
                    checked={settings.lockOnMinimize}
                    onChange={(e) => void save({lockOnMinimize: e.target.checked})}
                />
                {t('settings.lockOnMinimize')}
            </label>
        </Section>
    )
}

function SecuritySection() {
    const {t, i18n} = useTranslation()
    const showKit = useSession((s) => s.showKit)
    const helloEnrolled = useSession((s) => s.hello.enrolled)
    const syncOn = useSync((s) => s.status.phase !== 'off')
    const refreshHello = useSession((s) => s.refreshHello)
    const {settings, refresh} = useUi()
    const show = useToast((s) => s.show)
    const [current, setCurrent] = useState('')
    const [passwordError, setPasswordError] = useState<string | null>(null)
    const [prompt, setPrompt] = useState<'kit' | 'rotate' | null>(null)
    // Remounting the form clears the new-password fields after a change.
    const [formKey, setFormKey] = useState(0)

    const changePassword = async (newPassword: string) => {
        setPasswordError(null)
        try {
            await api.changePassword(current, newPassword)
            setCurrent('')
            setFormKey((k) => k + 1)
            show(t('settings.passwordChanged'))
        } catch (e) {
            setPasswordError(errorMessage(t, e))
            throw e
        }
    }

    const newKit = async (password: string) => {
        const {recoveryKey} = await api.newRecoveryKey(password)
        await refresh().catch(() => undefined)
        showKit(recoveryKey)
    }

    const rotate = async (password: string) => {
        // When Hello was on, the entry list offers to set it up again after the kit.
        const {recoveryKey} = await api.rotateVaultKey(password)
        await refresh().catch(() => undefined)
        await refreshHello().catch(() => undefined)
        showKit(recoveryKey)
    }

    return (
        <Section title={t('settings.security')}>
            <HelloSettings/>

            <hr className="my-6 border-neutral-200 dark:border-neutral-800"/>
            <h3 className="mb-3 font-medium">{t('settings.changePassword')}</h3>
            <Field label={t('settings.currentPassword')}>
                <PasswordInput value={current} onChange={(e) => setCurrent(e.target.value)}/>
            </Field>
            <NewPasswordForm
                key={formKey}
                label={t('settings.newPassword')}
                autoFocus={false}
                submitLabel={t('settings.changePasswordSubmit')}
                busyLabel={t('newPassword.saving')}
                onSubmit={changePassword}
                error={passwordError}
            />

            <hr className="my-6 border-neutral-200 dark:border-neutral-800"/>
            <h3 className="mb-1 font-medium">{t('kit.title')}</h3>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">
                {settings.lastKitAt
                    ? t('settings.kitLast', {date: new Date(settings.lastKitAt).toLocaleDateString(i18n.language)})
                    : t('settings.kitUnknown')}
            </p>
            <Button variant="secondary" onClick={() => setPrompt('kit')}>
                {t('settings.kitNew')}
            </Button>

            <hr className="my-6 border-neutral-200 dark:border-neutral-800"/>
            <h3 className="mb-1 font-medium">{t('settings.rotateTitle')}</h3>
            <p className="mb-3 text-sm text-neutral-600 dark:text-neutral-400">{t('settings.rotateIntro')}</p>
            <Button variant="danger" onClick={() => setPrompt('rotate')}>
                {t('settings.rotateButton')}
            </Button>

            {prompt === 'kit' && (
                <PasswordPrompt
                    title={t('settings.kitNew')}
                    confirmLabel={t('settings.kitNewConfirm')}
                    onConfirm={newKit}
                    onClose={() => setPrompt(null)}
                >
                    <p className="mb-4 text-sm text-neutral-600 dark:text-neutral-400">{t('settings.kitNewExplain')}</p>
                </PasswordPrompt>
            )}
            {prompt === 'rotate' && (
                <PasswordPrompt
                    title={t('settings.rotateTitle')}
                    confirmLabel={t('settings.rotateButton')}
                    danger
                    onConfirm={rotate}
                    onClose={() => setPrompt(null)}
                >
                    <ul className="mb-4 list-disc space-y-1 pl-5 text-sm text-neutral-600 dark:text-neutral-400">
                        {(t('settings.rotateExplain', {returnObjects: true}) as string[]).map((line, i) => (
                            <li key={i}>{line}</li>
                        ))}
                        {helloEnrolled && <li>{t('settings.rotateExplainHello')}</li>}
                        {syncOn && <li>{t('settings.rotateExplainSync')}</li>}
                    </ul>
                </PasswordPrompt>
            )}
        </Section>
    )
}

function AboutSection() {
    const {t} = useTranslation()
    const show = useToast((s) => s.show)
    const [info, setInfo] = useState<AppInfo | null>(null)

    useEffect(() => {
        api.appInfo().then(setInfo).catch((e) => console.error('app_info failed', e))
    }, [])

    return (
        <Section title={t('settings.about')}>
            <dl className="mb-4 grid grid-cols-[10rem_1fr] gap-y-1 text-sm">
                <dt className="text-neutral-500">{t('settings.version')}</dt>
                <dd>{info?.version ?? '…'}</dd>
                <dt className="text-neutral-500">{t('settings.vaultFormat')}</dt>
                <dd>{info ? `v${info.formatVersion}` : '…'}</dd>
                <dt className="text-neutral-500">{t('settings.license')}</dt>
                <dd>MIT</dd>
            </dl>
            <Button
                variant="secondary"
                onClick={() => api.openProjectPage().catch((e) => show(errorMessage(t, e), 4000))}
            >
                <ExternalLink className="size-4" aria-hidden/>
                {t('settings.projectPage')}
            </Button>
        </Section>
    )
}
