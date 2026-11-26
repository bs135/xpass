import {describe, expect, it} from 'vitest'
import {lostSecrets, move, newRow, rowsFromDetail, rowsProblem, toInput, MAX_FIELDS} from './fields'

const stored = [
    {name: 'PIN', value: null, secret: true, totp: false},
    {name: 'Branch', value: 'Main', secret: false, totp: false},
]

describe('custom field rows', () => {
    it('sends unchanged stored rows as keep, so secret values stay in Rust', () => {
        const rows = rowsFromDetail(stored)
        expect(toInput(rows)).toEqual([
            {name: 'PIN', secret: true, value: {keep: {index: 0, name: 'PIN'}}},
            {name: 'Branch', secret: false, value: {keep: {index: 1, name: 'Branch'}}},
        ])
    })

    it('keeps the stored name for keep after a rename, reorder and secret flip', () => {
        const [pin, branch] = rowsFromDetail(stored)
        const rows = [{...branch, name: 'Office', secret: true}, {...pin, name: 'Card PIN'}]
        expect(toInput(rows)).toEqual([
            {name: 'Office', secret: true, value: {keep: {index: 1, name: 'Branch'}}},
            {name: 'Card PIN', secret: true, value: {keep: {index: 0, name: 'PIN'}}},
        ])
        expect(lostSecrets(rows, stored)).toEqual([])
    })

    it('sends edited and new rows as set', () => {
        const [pin] = rowsFromDetail(stored)
        const rows = [{...pin, value: '9999', edited: true}, {...newRow(), name: 'New', value: 'x'}]
        expect(toInput(rows)).toEqual([
            {name: 'PIN', secret: true, value: {set: '9999'}},
            {name: 'New', secret: false, value: {set: 'x'}},
        ])
    })

    it('lists stored secrets that would be lost', () => {
        const [pin, branch] = rowsFromDetail(stored)
        expect(lostSecrets([branch], stored)).toEqual(['PIN'])
        expect(lostSecrets([{...pin, edited: true}, branch], stored)).toEqual(['PIN'])
        // A removed plain field is not a secret.
        expect(lostSecrets([pin], stored)).toEqual([])
    })

    it('finds problems before saving', () => {
        expect(rowsProblem(rowsFromDetail(stored))).toBeNull()
        expect(rowsProblem([{...newRow(), name: '  '}])).toBe('name')
        expect(rowsProblem([{...newRow(), name: 'n'.repeat(101)}])).toBe('tooLong')
        expect(rowsProblem(Array.from({length: MAX_FIELDS + 1}, () => ({...newRow(), name: 'x'})))).toBe('tooMany')
    })

    it('moves rows within bounds', () => {
        expect(move([1, 2, 3], 0, 1)).toEqual([2, 1, 3])
        expect(move([1, 2, 3], 2, 3)).toEqual([1, 2, 3])
        expect(move([1, 2, 3], 0, -1)).toEqual([1, 2, 3])
    })
})
