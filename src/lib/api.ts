/**
 * Typed wrappers around Tauri commands. All IPC goes through this module so the
 * contract with `src-tauri/src/commands` stays in one place.
 */
import {invoke} from '@tauri-apps/api/core'

export interface AppInfo {
    version: string
    formatVersion: number
}

export const api = {
    appInfo: () => invoke<AppInfo>('app_info'),
}
