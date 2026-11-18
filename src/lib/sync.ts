import type {ConnectionReport} from './api'

/** A test result that must not be saved, as an i18n key under `sync.report`, or null. */
export function blockingProblem(report: ConnectionReport): 'branchMissing' | 'otherVault' | 'notAVault' | null {
    if (!report.branchExists) return 'branchMissing'
    if (report.remote === 'otherVault') return 'otherVault'
    if (report.remote === 'notAVault') return 'notAVault'
    return null
}

/** The setup can be saved after this test: nothing blocking, and a public repository was acknowledged. */
export function canSave(report: ConnectionReport | null, publicAcknowledged: boolean): boolean {
    if (!report || blockingProblem(report)) return false
    return report.private || publicAcknowledged
}
