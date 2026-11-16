import {describe, expect, it} from 'vitest'
import {isTheme, resolveTheme} from './theme'

describe('resolveTheme', () => {
    it('follows the OS preference for system', () => {
        expect(resolveTheme('system', true)).toBe('dark')
        expect(resolveTheme('system', false)).toBe('light')
    })

    it('keeps an explicit choice regardless of the OS', () => {
        expect(resolveTheme('light', true)).toBe('light')
        expect(resolveTheme('dark', false)).toBe('dark')
    })
})

describe('isTheme', () => {
    it('accepts only known values', () => {
        expect(isTheme('dark')).toBe(true)
        expect(isTheme('blue')).toBe(false)
        expect(isTheme(null)).toBe(false)
    })
})
