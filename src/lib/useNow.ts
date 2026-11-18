import {useEffect, useState} from 'react'

/** Current time, refreshed every `intervalMs` (default 30 s), so "n min ago" texts stay current. */
export function useNow(intervalMs = 30_000): number {
    const [now, setNow] = useState(() => Date.now())
    useEffect(() => {
        const id = window.setInterval(() => setNow(Date.now()), intervalMs)
        return () => window.clearInterval(id)
    }, [intervalMs])
    return now
}
