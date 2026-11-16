export type Theme = 'system' | 'light' | 'dark'
export type ResolvedTheme = 'light' | 'dark'

export const THEMES: readonly Theme[] = ['system', 'light', 'dark']

export function isTheme(value: unknown): value is Theme {
    return typeof value === 'string' && (THEMES as readonly string[]).includes(value)
}

/** Resolves the user's choice against the OS preference. */
export function resolveTheme(theme: Theme, prefersDark: boolean): ResolvedTheme {
    if (theme === 'system') return prefersDark ? 'dark' : 'light'
    return theme
}

export function applyTheme(resolved: ResolvedTheme): void {
    const root = document.documentElement
    root.classList.toggle('dark', resolved === 'dark')
    root.classList.toggle('light', resolved === 'light')
}

export function systemPrefersDark(): boolean {
    return window.matchMedia('(prefers-color-scheme: dark)').matches
}
