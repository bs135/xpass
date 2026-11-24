import {describe, expect, it} from 'vitest'

import {bumpLevel, compare, nextVersion} from './release-prepare.mjs'

describe('release-prepare', () => {
    it('picks the bump level from Conventional Commits', () => {
        expect(bumpLevel(['docs: readme', 'chore(ci): x'])).toBe(0)
        expect(bumpLevel(['fix(core): a', 'docs: b'])).toBe(1)
        expect(bumpLevel(['perf: a'])).toBe(1)
        expect(bumpLevel(['fix: a', 'feat(ui): b'])).toBe(2)
        expect(bumpLevel(['feat!: drop format v1'])).toBe(3)
        expect(bumpLevel(['fix: a\n\nBREAKING CHANGE: new header'])).toBe(3)
        expect(bumpLevel(['Merge pull request #1', 'not conventional'])).toBe(0)
    })

    it('computes the next version, breaking changes stay minor before 1.0', () => {
        expect(nextVersion('0.5.0', 2)).toBe('0.6.0')
        expect(nextVersion('0.5.0', 1)).toBe('0.5.1')
        expect(nextVersion('0.5.3', 3)).toBe('0.6.0')
        expect(nextVersion('1.2.3', 3)).toBe('2.0.0')
        expect(nextVersion('1.2.3', 0)).toBeNull()
    })

    it('compares versions numerically', () => {
        expect(compare('0.10.0', '0.9.0')).toBeGreaterThan(0)
        expect(compare('0.6.0', '0.6.0')).toBe(0)
        expect(compare('0.5.1', '0.6.0')).toBeLessThan(0)
    })
})
