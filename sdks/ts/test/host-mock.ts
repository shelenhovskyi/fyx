// A recording stand-in for the `fyx:ui/host` import, shared by the source-level tests (through
// `register-host-mock.mjs`) and the component tests (as the instantiation import).

import type { LogLevel, Update } from 'fyx-ui-sdk';

export interface HostMock {
    readonly logs: Array<{ level: LogLevel; message: string }>;
    readonly pushes: Array<{ surfaceId: string; update: Update }>;
    log(level: LogLevel, message: string): void;
    pushUpdate(surfaceId: string, update: Update): void;
}

export function createHostMock(): HostMock {
    const logs: HostMock['logs'] = [];
    const pushes: HostMock['pushes'] = [];
    return {
        logs,
        pushes,
        log(level, message) {
            logs.push({ level, message });
        },
        pushUpdate(surfaceId, update) {
            pushes.push({ surfaceId, update });
        },
    };
}

declare global {
    var fyxHostMock: HostMock | undefined;
}

/** Installs a fresh mock as the host that SDK sources run under Node talk to, and returns it. */
export function installHostMock(): HostMock {
    const host = createHostMock();
    globalThis.fyxHostMock = host;
    return host;
}
