import type {EntrySummary, HealthReport} from './api'

/** Entries in the health report, in list order, limited to `visible` (search). */
export function healthSections(
    visible: EntrySummary[],
    report: HealthReport,
): {weak: EntrySummary[]; reused: EntrySummary[][]} {
    const byId = new Map(visible.map((e) => [e.id, e]))
    const pick = (ids: string[]) =>
        ids.flatMap((id) => (byId.has(id) ? [byId.get(id)!] : [])).sort((a, b) => visible.indexOf(a) - visible.indexOf(b))
    return {
        weak: pick(report.weak),
        // A group shows while at least two of its entries match the search.
        reused: report.reused.map(pick).filter((g) => g.length > 1),
    }
}

/** Every entry the report flags. */
export function flaggedIds(report: HealthReport): Set<string> {
    return new Set([...report.weak, ...report.reused.flat()])
}

/** What the entry view shows next to the password. */
export function healthOf(id: string, report: HealthReport | null): {weak: boolean; reusedWith: number} {
    if (!report) return {weak: false, reusedWith: 0}
    const group = report.reused.find((g) => g.includes(id))
    return {weak: report.weak.includes(id), reusedWith: group ? group.length - 1 : 0}
}
