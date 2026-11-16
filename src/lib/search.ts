import type {EntrySummary} from './api'

/**
 * Filters entries by a search query over metadata (title, username, URL host, tags).
 * Every whitespace-separated term must match somewhere, case-insensitively.
 */
export function filterEntries(entries: EntrySummary[], query: string, favoritesOnly: boolean): EntrySummary[] {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
    return entries.filter((e) => {
        if (favoritesOnly && !e.favorite) return false
        if (terms.length === 0) return true
        const haystack = [e.title, e.username, e.urlHost ?? '', ...e.tags].join('\n').toLowerCase()
        return terms.every((t) => haystack.includes(t))
    })
}

/** Splits a comma-separated tag input into trimmed, non-empty tags. */
export function parseTags(input: string): string[] {
    return input
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean)
}

/** Splits a multi-line URL input into trimmed, non-empty lines. */
export function parseLines(input: string): string[] {
    return input
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter(Boolean)
}
