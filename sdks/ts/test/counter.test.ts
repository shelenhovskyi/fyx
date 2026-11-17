// The counter contract against the TypeScript sources, without building a component.

import { test } from 'node:test';

import { registerExtension } from 'fyx-ui-sdk';
import { Counter } from '../demo-counter/src/counter.ts';
import { replayCounterContract } from './counter-contract.ts';
import { installHostMock } from './host-mock.ts';

test('counter sources behave as documented', () => {
    const host = installHostMock();
    replayCounterContract(registerExtension(() => new Counter()), host);
});
