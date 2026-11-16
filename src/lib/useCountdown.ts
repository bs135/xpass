import {useEffect, useState} from 'react'

/**
 * Seconds left until `until` (epoch ms), updated every 250 ms; 0 once passed.
 * Used for the wrong-password delay.
 */
export function useCountdown(until: number | null): number {
    const [now, setNow] = useState(() => Date.now())
    useEffect(() => {
        if (until === null) return
        const id = window.setInterval(() => setNow(Date.now()), 250)
        return () => window.clearInterval(id)
    }, [until])
    return until === null ? 0 : Math.max(0, Math.ceil((until - now) / 1000))
}
