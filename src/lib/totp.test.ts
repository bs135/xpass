import {describe, expect, it} from 'vitest'
import {formatCode, secondsLeft} from './totp'

describe('totp display', () => {
    it('groups codes', () => {
        expect(formatCode('287082')).toBe('287 082')
        expect(formatCode('9428708')).toBe('942 8708')
        expect(formatCode('94287082')).toBe('9428 7082')
        expect(formatCode('')).toBe('')
    })

    it('counts seconds down to zero', () => {
        expect(secondsLeft(10_000, 0)).toBe(10)
        expect(secondsLeft(10_000, 9_001)).toBe(1)
        expect(secondsLeft(10_000, 12_000)).toBe(0)
    })
})
