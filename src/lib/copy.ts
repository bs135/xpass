import type {TFunction} from 'i18next'
import {useToast} from '../stores/toast'
import {api, CLIPBOARD_CLEAR_SECONDS, type FieldRef} from './api'
import {errorMessage} from './errors'

/** Copies an entry field through Rust (secure clipboard) and shows a toast. */
export async function copyField(t: TFunction, id: string, field: FieldRef): Promise<void> {
    await runCopy(t, () => api.copyField(id, field), field.kind === 'password' || field.kind === 'field')
}

/** Copies a generated password through Rust (secure clipboard) and shows a toast. */
export async function copyText(t: TFunction, text: string): Promise<void> {
    await runCopy(t, () => api.copyText(text), true)
}

async function runCopy(t: TFunction, copy: () => Promise<void>, secret: boolean): Promise<void> {
    const {show} = useToast.getState()
    try {
        await copy()
        show(secret ? t('copy.secret', {seconds: CLIPBOARD_CLEAR_SECONDS}) : t('copy.done'))
    } catch (e) {
        show(errorMessage(t, e), 4000)
    }
}
