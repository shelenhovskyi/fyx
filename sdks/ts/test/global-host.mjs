// Forwards to whatever `installHostMock` put in `globalThis.fyxHostMock` at call time.

function currentHost() {
    const host = globalThis.fyxHostMock;
    if (host === undefined) {
        throw new Error('fyx:ui/host called before installHostMock()');
    }
    return host;
}

export function log(level, message) {
    currentHost().log(level, message);
}

export function pushUpdate(surfaceId, update) {
    currentHost().pushUpdate(surfaceId, update);
}
