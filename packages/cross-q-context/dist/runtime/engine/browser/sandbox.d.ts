import { QuickJsEngine } from '../engine.js';
import type { QuickJsHostConfig } from '../engine.js';
import type { SendRequestHost } from '../host-types.js';
/**
 * The browser's reduced require chain.
 *
 * Two tiers, both Node-free, and deliberately no third:
 * 1. **vendor IIFEs** — build-time bundles of the built-in packages. This is the
 *    load-bearing one: Chai arrives here, and `rq.test()` / `rq.expect()` are built
 *    on it. A host without this tier can run scripts but cannot assert.
 * 2. **`needs_bridge` globals** — `require('crypto')` → the `__rq_cryptoModule`
 *    global the shared shim already installed.
 *
 * Anything else throws, which is the TB's S1 boundary: `require('lodash')` fails on
 * web with a message naming the capability, while `require('chai')` works.
 */
export declare function createBrowserRequireSupport(): {
    isolateShim: string;
    prepare: () => Promise<{
        resolve: (id: string) => unknown;
    }>;
};
/**
 * Build the browser host config.
 *
 * `sendRequestHost` is **required**: a script's `fetch` must be delegated, never
 * issued from the browser directly (ADR-202 puts web egress in the cloud). Making
 * it a parameter rather than an optional field keeps that unrepresentable.
 */
export declare function createBrowserQuickJsHost(sendRequestHost: SendRequestHost): QuickJsHostConfig;
/**
 * The Safe engine, wired to the browser host. Same engine as desktop — only the
 * config differs.
 */
export declare class BrowserQuickJsSandbox extends QuickJsEngine {
    constructor(sendRequestHost: SendRequestHost);
}
