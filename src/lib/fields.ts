import type {EntryDetail, FieldInput} from './api'

/** Same limits as `src-tauri/src/entries.rs` (v0.8 plan V6). */
export const MAX_FIELDS = 50
export const MAX_FIELD_NAME = 100
export const MAX_FIELD_VALUE = 10_000

/**
 * One row of the custom field editor. A row loaded from the entry remembers where
 * it came from (`index`, `storedName`), so an unchanged value is sent as `keep`:
 * a secret value never has to reach the WebView to stay as it is.
 */
export interface FieldRow {
    /** Local React key. */
    key: number
    name: string
    secret: boolean
    /** Typed or revealed value; for a stored secret not revealed yet: ''. */
    value: string
    /** The stored field this row came from, or null for a new row. */
    stored: {index: number; name: string; secret: boolean} | null
    /** The user replaced or typed the value (send `set`). */
    edited: boolean
    /** A stored secret value is in `value` (revealed for editing). */
    revealed: boolean
}

let nextKey = 1

export function rowsFromDetail(fields: EntryDetail['fields']): FieldRow[] {
    return fields.map((f, index) => ({
        key: nextKey++,
        name: f.name,
        secret: f.secret,
        value: f.value ?? '',
        stored: {index, name: f.name, secret: f.secret},
        edited: false,
        revealed: false,
    }))
}

export function newRow(): FieldRow {
    return {key: nextKey++, name: '', secret: false, value: '', stored: null, edited: true, revealed: false}
}

/** The `fields` of `save_entry`. */
export function toInput(rows: FieldRow[]): FieldInput[] {
    return rows.map((r) =>
        r.stored && !r.edited
            ? {name: r.name, secret: r.secret, value: {keep: {index: r.stored.index, name: r.stored.name}}}
            : {name: r.name, secret: r.secret, value: {set: r.value}},
    )
}

/**
 * Stored secret fields whose value would be lost by saving (changed or removed):
 * custom fields have no history (V7), so the form asks first.
 */
export function lostSecrets(rows: FieldRow[], stored: EntryDetail['fields']): string[] {
    const kept = new Set(rows.filter((r) => r.stored && !r.edited).map((r) => r.stored!.index))
    return stored.flatMap((f, index) => (f.secret && !kept.has(index) ? [f.name] : []))
}

/** First problem the form can see before Rust checks it; null when fine. */
export function rowsProblem(rows: FieldRow[]): 'name' | 'tooMany' | 'tooLong' | null {
    if (rows.length > MAX_FIELDS) return 'tooMany'
    if (rows.some((r) => !r.name.trim())) return 'name'
    if (rows.some((r) => [...r.name.trim()].length > MAX_FIELD_NAME || [...r.value].length > MAX_FIELD_VALUE)) {
        return 'tooLong'
    }
    return null
}

export function move<T>(list: T[], from: number, to: number): T[] {
    if (to < 0 || to >= list.length) return list
    const next = [...list]
    const [item] = next.splice(from, 1)
    next.splice(to, 0, item)
    return next
}
