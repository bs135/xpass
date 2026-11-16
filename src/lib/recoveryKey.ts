/** Helpers for showing and confirming the printable recovery key (`XXXXX-XXXXX-...`). */

export function splitGroups(key: string): string[] {
    return key.split('-').filter(Boolean)
}

/**
 * Normalizes one typed group the same way the Rust decoder does: case-insensitive,
 * spaces and dashes ignored, `0`/`1`/`8` read as `O`/`I`/`B`.
 */
export function normalizeGroup(input: string): string {
    return input
        .toUpperCase()
        .replace(/[\s-]/g, '')
        .replace(/0/g, 'O')
        .replace(/1/g, 'I')
        .replace(/8/g, 'B')
}

/** Picks `n` distinct group indices (0-based, ascending) for the confirmation step. */
export function pickConfirmGroups(groupCount: number, n = 2, random: () => number = Math.random): number[] {
    const pool = Array.from({length: groupCount}, (_, i) => i)
    const picked: number[] = []
    while (picked.length < Math.min(n, groupCount)) {
        const i = Math.floor(random() * pool.length)
        picked.push(pool.splice(i, 1)[0])
    }
    return picked.sort((a, b) => a - b)
}
