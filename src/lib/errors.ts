import type {TFunction} from 'i18next'
import {type AppError, isAppError} from './api'

/** Translated message for a command error. Never includes secret data. */
export function errorMessage(t: TFunction, e: unknown): string {
    if (!isAppError(e)) return t('errors.generic')
    switch (e.code) {
        case 'throttled':
            return t('errors.throttled', {seconds: Math.ceil(e.retryAfterMs / 1000)})
        case 'unsupported_version':
            return t('errors.unsupported_version', {version: e.version})
        case 'unsupported_schema':
            return t('errors.unsupported_schema', {schema: e.schema})
        case 'sync_rate_limited':
            return e.retryAfterS
                ? t('errors.sync_rate_limited_wait', {minutes: Math.max(1, Math.ceil(e.retryAfterS / 60))})
                : t('errors.sync_rate_limited')
        case 'import_too_large':
            return t('errors.import_too_large', {mb: e.maxMb, rows: e.maxRows.toLocaleString()})
        case 'import_invalid_csv':
            return e.line === null
                ? t('errors.import_invalid_csv')
                : t('errors.import_invalid_csv_line', {line: e.line})
        case 'field_limit':
            return t('errors.field_limit', {fields: e.maxFields, name: e.maxName, value: e.maxValue.toLocaleString()})
        case 'invalid_input':
            return e.field === 'fieldName'
                ? t('errors.field_name')
                : t('errors.invalid_input', {defaultValue: t('errors.generic')})
        case 'io':
            return t('errors.io', {kind: e.kind})
        default:
            return t(`errors.${e.code}`, {defaultValue: t('errors.generic')})
    }
}

/** Milliseconds to wait when the error is a throttle, otherwise null. */
export function throttleDelay(e: unknown): number | null {
    return isAppError(e) && e.code === 'throttled' ? e.retryAfterMs : null
}

export type {AppError}
