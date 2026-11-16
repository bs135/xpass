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
