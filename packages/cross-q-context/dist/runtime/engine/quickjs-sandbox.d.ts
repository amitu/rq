/**
 * `QuickJsSandbox` — the **Node host** for the Safe engine (ADR-204).
 *
 * The engine itself now lives in `./engine.ts` and is host-agnostic: it takes a
 * `QuickJsHostConfig` and imports no `node:*`. This file is the thin Node half —
 * it supplies the CJS QuickJS variant, the four Node-backed capability bridges,
 * the Node fetch bridge (which keeps its direct `globalThis.fetch` fallback), and
 * the full `require()` chain including the SOURCE_BUNDLE tier.
 *
 * The class keeps its original name and constructor shape deliberately, so every
 * existing construction site — and all 33 test files — are unchanged. That is what
 * made the extraction verifiable: the whole suite passes with zero test edits.
 *
 * The browser half is `@requestly/sandbox-browser`, which supplies a different
 * config to the same engine.
 */
import { QuickJsEngine } from './engine.js';
import type { QuickJsHostConfig } from './engine.js';
import type { BundleCache } from './isolated/source-bundler.js';
import type { SafePackageResolver } from '../index.js';
/** The Node host's config — everything the engine used to import directly. */
export declare const NODE_QUICKJS_HOST: QuickJsHostConfig;
/**
 * The Safe engine, wired to the Node host. Constructor signature is unchanged from
 * before the ADR-204 extraction.
 */
export declare class QuickJsSandbox extends QuickJsEngine {
    constructor(resolver?: SafePackageResolver, bundleCache?: BundleCache);
}
