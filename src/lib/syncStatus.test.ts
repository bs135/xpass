import {describe, expect, it} from 'vitest'
import type {SyncStatus} from './api'
import {minutesSince, syncView} from './syncStatus'

const base: SyncStatus = {phase: 'idle', dirty: false, lastSyncAt: null, error: null, passwordChanged: false}

describe('sync indicator', () => {
    it('is hidden while sync is off', () => {
        expect(syncView({...base, phase: 'off'})).toBeNull()
    })

    it('tells synced from pending changes', () => {
        expect(syncView(base)?.key).toBe('synced')
        expect(syncView({...base, dirty: true})?.key).toBe('pending')
    })

    it('sends user problems to Settings and retries the rest', () => {
        for (const phase of ['tokenInvalid', 'needsPassword', 'differentVault'] as const) {
            expect(syncView({...base, phase})).toEqual({key: phase, action: 'openSettings', warn: true})
        }
        expect(syncView({...base, phase: 'offline'})?.action).toBe('syncNow')
        expect(syncView({...base, phase: 'error'})).toEqual({key: 'error', action: 'syncNow', warn: true})
    })

    it('counts whole minutes since the last sync', () => {
        expect(minutesSince(null, 1_000)).toBeNull()
        expect(minutesSince(0, 59_999)).toBe(0)
        expect(minutesSince(0, 125_000)).toBe(2)
        expect(minutesSince(10_000, 0)).toBe(0)
    })
})
