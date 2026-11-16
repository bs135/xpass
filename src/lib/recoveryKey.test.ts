import {describe, expect, it} from 'vitest'
import {normalizeGroup, pickConfirmGroups, splitGroups} from './recoveryKey'

describe('splitGroups', () => {
    it('splits on dashes', () => {
        expect(splitGroups('ABCDE-FGHIJ-KLM')).toEqual(['ABCDE', 'FGHIJ', 'KLM'])
    })
})

describe('normalizeGroup', () => {
    it('ignores case and spaces and maps look-alike digits', () => {
        expect(normalizeGroup(' ab cde ')).toBe('ABCDE')
        expect(normalizeGroup('o1b80')).toBe('OIBBO')
    })
})

describe('pickConfirmGroups', () => {
    it('returns distinct sorted indices in range', () => {
        for (let run = 0; run < 100; run++) {
            const picked = pickConfirmGroups(11)
            expect(picked).toHaveLength(2)
            expect(picked[0]).toBeLessThan(picked[1])
            expect(picked[1]).toBeLessThan(11)
        }
    })

    it('is deterministic with a fixed random source', () => {
        expect(pickConfirmGroups(11, 2, () => 0)).toEqual([0, 1])
        expect(pickConfirmGroups(11, 2, () => 0.999)).toEqual([9, 10])
    })

    it('never asks for more groups than exist', () => {
        expect(pickConfirmGroups(1)).toEqual([0])
    })
})
