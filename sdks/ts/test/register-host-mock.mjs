// Loaded with `node --import`: resolves the WIT import `fyx:ui/host@0.1.0`, which only exists inside
// a component, to `global-host.mjs`, so the SDK and demo sources run unmodified under Node.

import { register } from 'node:module';

register('./host-resolve-hook.mjs', import.meta.url);
