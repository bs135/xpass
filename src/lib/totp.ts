/** Groups a code for reading: 123 456, 1234 5678, 123 4567. */
export function formatCode(code: string): string {
    const head = Math.floor(code.length / 2)
    return code.length < 6 ? code : `${code.slice(0, head)} ${code.slice(head)}`
}

/** Seconds left, rounded up, never below 0. */
export function secondsLeft(expiresAt: number, now: number): number {
    return Math.max(0, Math.ceil((expiresAt - now) / 1000))
}
