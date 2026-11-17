const globalHost = new URL('./global-host.mjs', import.meta.url).href;

export async function resolve(specifier, context, nextResolve) {
    if (specifier === 'fyx:ui/host@0.1.0') {
        return { url: globalHost, shortCircuit: true };
    }
    return nextResolve(specifier, context);
}
