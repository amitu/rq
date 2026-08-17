import { EXTERNAL_BUILTIN_PACKAGES } from '../../index.js';
// The guest realm — shim strings, the bridge factory, the vendor IIFEs — comes
// from the package neither host owns (ADR-217). A second copy of any of it is how
// the two surfaces silently become two products.
import {
  BUFFER_ISOLATE_SHIM,
  CONSOLE_ISOLATE_SHIM,
  CRYPTO_ISOLATE_SHIM,
  FETCH_ISOLATE_SHIM,
  PROCESS_ISOLATE_SHIM,
  STREAM_ISOLATE_SHIM,
  UTIL_ISOLATE_SHIM,
  VENDOR_IIFES,
  ZLIB_ISOLATE_SHIM,
  createSafeBridge,
} from '../index.js';
// The residue the Node package still owns: the engine class itself and the
// require() chain's guest-side data. Reached ONLY via the Node-free `/shared`
// subpath — the root and `/isolated` both drag `node:*` into a browser bundle.
import { QuickJsEngine } from '../engine.js';
import { NEEDS_BRIDGE_MODULE_GLOBALS } from '../isolated/needs-bridge-globals.js';
import { REQUIRE_ISOLATE_SHIM } from '../isolated/shims/require.shim.js';

import { browserBufferHandler } from './bridges/buffer.js';
import { browserCryptoHandler } from './bridges/crypto.js';
import { createBrowserFetchBridge } from './bridges/fetch.js';
import { browserUtilHandler } from './bridges/util.js';
import { browserZlibHandler } from './bridges/zlib.js';
import { getQuickJsModule } from './quickjs-module.js';

import type { SafeBridge } from '../index.js';
import type { QuickJsHostConfig } from '../engine.js';
import type { SendRequestHost } from '../host-types.js';

/**
 * `BrowserQuickJsSandbox` — the **browser host** for the Safe engine (ADR-204).
 *
 * The engine is the identical `QuickJsEngine` desktop runs; this file is only its
 * configuration. That is the whole point of "one engine, two hosts": script
 * semantics cannot fork, because there is one implementation.
 *
 * What differs from the Node host, and only this:
 *
 * | Seam | Node | Browser |
 * |---|---|---|
 * | QuickJS variant | `-cjs-` | `-browser-` (same pinned version) |
 * | buffer/crypto/util/zlib | `node:*` handlers | pure-JS handlers in `./bridges/` |
 * | fetch | direct or delegated | **delegated only** — required host |
 * | require tiers | vendor + bridge + SOURCE_BUNDLE | vendor + bridge (no bundler) |
 *
 * The in-isolate shims are shared **verbatim**, in the same order.
 */

/** The in-isolate shim order. Must match the Node host's `ISOLATE_SHIMS` exactly. */
const BROWSER_ISOLATE_SHIMS: readonly string[] = [
  CONSOLE_ISOLATE_SHIM,
  PROCESS_ISOLATE_SHIM,
  BUFFER_ISOLATE_SHIM,
  CRYPTO_ISOLATE_SHIM,
  UTIL_ISOLATE_SHIM,
  STREAM_ISOLATE_SHIM,
  ZLIB_ISOLATE_SHIM,
  FETCH_ISOLATE_SHIM,
];

const vendorIifeLookup: Readonly<Record<string, string>> = VENDOR_IIFES;

/**
 * `require` id → the global the package's build-time IIFE assigns itself to
 * (`chai` → `__chai`), sourced from the shared package registry.
 *
 * The IIFE is a `var __name = (()=>{...})()` string. The in-isolate require shim
 * indirect-`eval`s it and then reads `globalThis[globalName]` — so WITHOUT the right
 * `globalName` the eval succeeds, the global is set, and the shim returns `undefined`.
 * That failure is silent and downstream: `require('chai')` yields `undefined`, so
 * `__rq_chai` is `undefined`, so `rq.expect` is `undefined`, and every `rq.test()`
 * fails with "not a function" — with nothing pointing back here.
 *
 * Read from `EXTERNAL_BUILTIN_PACKAGES` rather than hardcoded so this table cannot
 * drift from the one the Node host resolves against.
 */
const vendorGlobalNames: Readonly<Record<string, string>> = Object.fromEntries(
  EXTERNAL_BUILTIN_PACKAGES.map((pkg) => [pkg.id, pkg.globalName]),
);

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
export function createBrowserRequireSupport(): {
  isolateShim: string;
  prepare: () => Promise<{ resolve: (id: string) => unknown }>;
} {
  return {
    isolateShim: REQUIRE_ISOLATE_SHIM,
    // Nothing to pre-bundle: both supported tiers resolve from static tables, so
    // unlike Node there is no async bundling pass before the guest runs.
    prepare: () =>
      Promise.resolve({
        resolve: (id: string): unknown => {
          const bareId = id.replace(/^node:/, '');

          const bridgeGlobal = NEEDS_BRIDGE_MODULE_GLOBALS[id] ?? NEEDS_BRIDGE_MODULE_GLOBALS[bareId];
          if (bridgeGlobal !== undefined) return { kind: 'bridge', global: bridgeGlobal };

          // `kind: 'iife'` + `code` + `globalName` is the shape the shared
          // `REQUIRE_ISOLATE_SHIM` destructures (see
          // `sandbox-node/src/isolated/shims/require.shim.ts`).
          // Any other shape falls through the shim's final `else` to
          // `evalModule(res.code)` with `code === undefined`, which returns `undefined`
          // instead of throwing — the silent path described on `vendorGlobalNames`.
          const iife = vendorIifeLookup[id] ?? vendorIifeLookup[bareId];
          const globalName = vendorGlobalNames[id] ?? vendorGlobalNames[bareId];
          if (iife !== undefined && globalName !== undefined) {
            return { kind: 'iife', code: iife, globalName };
          }

          // STATIC message (`gr-static-error-messages`) — a require id is unbounded
          // user input and must not be interpolated, or Sentry grouping explodes.
          // FR-10 asks the error to name the CAPABILITY, which it does; the guest's
          // own stack already shows which `require()` call raised it.
          throw new Error(
            "Installing npm packages isn't available for scripts in the browser. " +
              'The built-in packages (chai, lodash, moment, uuid, ajv, cheerio, xml2js, csv-parse) work here; ' +
              'for anything else, run this request in the desktop app.',
          );
        },
      }),
  };
}

/**
 * Build the browser host config.
 *
 * `sendRequestHost` is **required**: a script's `fetch` must be delegated, never
 * issued from the browser directly (ADR-202 puts web egress in the cloud). Making
 * it a parameter rather than an optional field keeps that unrepresentable.
 */
export function createBrowserQuickJsHost(sendRequestHost: SendRequestHost): QuickJsHostConfig {
  return {
    createModule: getQuickJsModule,
    createRequireSupport: () => createBrowserRequireSupport(),
    valueBridgeFactories: [
      (): SafeBridge => createSafeBridge('__rq_buffer', browserBufferHandler),
      (): SafeBridge => createSafeBridge('__rq_crypto', browserCryptoHandler),
      (): SafeBridge => createSafeBridge('__rq_util_inspect', browserUtilHandler),
      (): SafeBridge => createSafeBridge('__rq_zlib', browserZlibHandler),
    ],
    createFetchBridge: () => createBrowserFetchBridge(sendRequestHost),
    isolateShims: BROWSER_ISOLATE_SHIMS,
  };
}

/**
 * The Safe engine, wired to the browser host. Same engine as desktop — only the
 * config differs.
 */
export class BrowserQuickJsSandbox extends QuickJsEngine {
  constructor(sendRequestHost: SendRequestHost) {
    // No package resolver and no bundle cache: the SOURCE_BUNDLE tier is S2.
    super(undefined, undefined, createBrowserQuickJsHost(sendRequestHost));
  }
}
