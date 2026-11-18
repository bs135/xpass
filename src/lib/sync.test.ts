import {describe, expect, it} from 'vitest'
import type {ConnectionReport} from './api'
import {blockingProblem, canSave} from './sync'

const ok: ConnectionReport = {private: true, branch: 'main', branchExists: true, remote: 'missing'}

describe('sync setup checks', () => {
    it('allows a private repository with no vault or the same vault', () => {
        expect(canSave(ok, false)).toBe(true)
        expect(canSave({...ok, remote: 'sameVault'}, false)).toBe(true)
    })

    it('needs a test result first', () => {
        expect(canSave(null, true)).toBe(false)
    })

    it('blocks a missing branch, another vault or a foreign file', () => {
        expect(blockingProblem({...ok, branchExists: false})).toBe('branchMissing')
        expect(blockingProblem({...ok, remote: 'otherVault'})).toBe('otherVault')
        expect(blockingProblem({...ok, remote: 'notAVault'})).toBe('notAVault')
        expect(canSave({...ok, remote: 'otherVault'}, true)).toBe(false)
    })

    it('asks to acknowledge a public repository', () => {
        const pub = {...ok, private: false}
        expect(canSave(pub, false)).toBe(false)
        expect(canSave(pub, true)).toBe(true)
    })
})
