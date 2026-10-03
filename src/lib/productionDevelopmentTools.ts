/** Vite release-only aliases. These modules have no listeners, state or IPC. */
export function observeSessionWireReceived(): void {}
export function observeSessionStoreApplied(): void {}
export function DevelopmentDebugger(): null { return null; }
export function DevelopmentOverlayTrace(): null { return null; }
