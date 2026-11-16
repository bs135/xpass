import {describe, expect, it} from 'vitest'
import type {EntrySummary} from './api'
import {filterEntries, parseLines, parseTags} from './search'

const entry = (title: string, extra: Partial<EntrySummary> = {}): EntrySummary => ({
    id: title,
    title,
    username: '',
    urlHost: null,
    tags: [],
    favorite: false,
    updatedAt: 0,
    ...extra,
})

const entries = [
    entry('GitHub', {username: 'alice', urlHost: 'github.com', tags: ['work'], favorite: true}),
    entry('Bank', {username: 'alice@mail.com', tags: ['finance']}),
    entry('Mail', {urlHost: 'mail.example.com'}),
]

const titles = (list: EntrySummary[]) => list.map((e) => e.title)

describe('filterEntries', () => {
    it('returns everything for an empty query', () => {
        expect(titles(filterEntries(entries, '  ', false))).toEqual(['GitHub', 'Bank', 'Mail'])
    })

    it('matches title, username, host and tags case-insensitively', () => {
        expect(titles(filterEntries(entries, 'GITHUB', false))).toEqual(['GitHub'])
        expect(titles(filterEntries(entries, 'alice', false))).toEqual(['GitHub', 'Bank'])
        expect(titles(filterEntries(entries, 'example.com', false))).toEqual(['Mail'])
        expect(titles(filterEntries(entries, 'finance', false))).toEqual(['Bank'])
    })

    it('requires every term to match', () => {
        expect(titles(filterEntries(entries, 'alice work', false))).toEqual(['GitHub'])
        expect(titles(filterEntries(entries, 'alice nothing', false))).toEqual([])
    })

    it('filters favorites', () => {
        expect(titles(filterEntries(entries, '', true))).toEqual(['GitHub'])
        expect(titles(filterEntries(entries, 'bank', true))).toEqual([])
    })
})

describe('parsers', () => {
    it('parses tags and lines', () => {
        expect(parseTags(' work, ,2fa ,')).toEqual(['work', '2fa'])
        expect(parseLines('a.com\r\n\n  b.com  \n')).toEqual(['a.com', 'b.com'])
    })
})
