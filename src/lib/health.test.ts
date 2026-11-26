import {describe, expect, it} from 'vitest'
import type {EntrySummary, HealthReport} from './api'
import {flaggedIds, healthOf, healthSections} from './health'

const entry = (id: string): EntrySummary => ({
    id,
    title: id.toUpperCase(),
    username: '',
    urlHost: null,
    tags: [],
    favorite: false,
    updatedAt: 0,
})
const list = ['a', 'b', 'c', 'd', 'e'].map(entry)
const report: HealthReport = {weak: ['d', 'b'], reused: [['e', 'a'], ['b', 'c']], checkedAt: 1}

describe('password health', () => {
    it('groups entries in list order', () => {
        const s = healthSections(list, report)
        expect(s.weak.map((e) => e.id)).toEqual(['b', 'd'])
        expect(s.reused.map((g) => g.map((e) => e.id))).toEqual([
            ['a', 'e'],
            ['b', 'c'],
        ])
    })

    it('follows the search', () => {
        const s = healthSections([list[1], list[2], list[4]], report)
        expect(s.weak.map((e) => e.id)).toEqual(['b'])
        expect(s.reused.map((g) => g.map((e) => e.id))).toEqual([['b', 'c']])
    })

    it('flags and describes entries', () => {
        expect([...flaggedIds(report)].sort()).toEqual(['a', 'b', 'c', 'd', 'e'])
        expect(healthOf('b', report)).toEqual({weak: true, reusedWith: 1})
        expect(healthOf('a', report)).toEqual({weak: false, reusedWith: 1})
        expect(healthOf('x', report)).toEqual({weak: false, reusedWith: 0})
        expect(healthOf('b', null)).toEqual({weak: false, reusedWith: 0})
    })
})
