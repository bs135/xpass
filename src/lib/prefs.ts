/**
 * Per-device UI preferences (theme, language) in localStorage.
 *
 * Only non-sensitive values belong here. Storage may be unavailable, so every
 * access is guarded and callers always get a usable default.
 * T9 moves these into the Rust-side settings.json.
 */
export function readPref(key: string): string | null {
    try {
        return window.localStorage.getItem(`xpass.${key}`)
    } catch {
        return null
    }
}

export function writePref(key: string, value: string): void {
    try {
        window.localStorage.setItem(`xpass.${key}`, value)
    } catch {
        // Ignore: preferences are a convenience.
    }
}
