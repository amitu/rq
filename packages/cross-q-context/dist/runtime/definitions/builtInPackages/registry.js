/**
 * Single source of truth for external built-in packages available via require().
 * These are IIFE-bundled npm packages, shown in the Packages UI dropdown.
 *
 * Adding a new external built-in package:
 * 1. Add an entry here
 * 2. Add the devDependency to both packages/sandbox-definitions and modules/sandbox-node
 * 3. The IIFE generation and codegen pick it up automatically
 * 4. If the package doesn't ship own types (no `types` field in its package.json),
 *    add `typesPackage` and `typesVersion` pointing to the correct @types/ package
 */
export const EXTERNAL_BUILTIN_PACKAGES = [
    // All ten are pure-JS in their bundled form → SOURCE_BUNDLE: esbuild bundles
    // each host-side and the isolate eval's the bundle with no host capability
    // (ADR-010 §10/§36 — the PoC verified these run inside the isolate).
    // uuid/cheerio/crypto-js touch crypto/Buffer-shaped internals only via the
    // NEEDS_BRIDGE Node built-ins the bundle then require()s (crypto-js reads
    // `crypto.getRandomValues`, which the Safe crypto bridge installs) — the
    // package itself carries no host reference.
    // `handlebars` is `internal: true` — delivered into both guests via VENDOR_IIFES
    // but suppressed from the user-facing require() codegen (ADR-202 Decision 2).
    { id: 'moment', entry: 'moment', globalName: '__moment', version: '2.30.1', safeModeClass: 'source_bundle' },
    {
        id: 'xml2js',
        entry: 'xml2js',
        globalName: '__xml2js',
        version: '0.6.2',
        typesPackage: '@types/xml2js',
        typesVersion: '0.4.14',
        safeModeClass: 'source_bundle',
    },
    {
        id: 'uuid',
        entry: 'uuid',
        globalName: '__uuid',
        version: '11.1.0',
        typesPackage: '@types/uuid',
        typesVersion: '10.0.0',
        safeModeClass: 'source_bundle',
    },
    {
        id: 'csv-parse/lib/sync',
        entry: 'csv-parse/sync',
        globalName: '__csv_parse',
        version: '5.6.0',
        safeModeClass: 'source_bundle',
    },
    { id: 'cheerio', entry: 'cheerio', globalName: '__cheerio', version: '1.0.0', safeModeClass: 'source_bundle' },
    // Postman injects `CryptoJS` as a bare global (crypto-js@3.3.0), so imported
    // Postman scripts reference it with no require line — RQ-5512. Pure JS with no
    // host capability, so SOURCE_BUNDLE: bundled host-side and eval'd in-isolate
    // with no bridge. Ships no `types` field of its own → @types override required.
    // The bare `CryptoJS` global is installed lazily over this entry by
    // CONVENIENCE_GLOBALS_SHIM, so both spellings share one bundle.
    {
        id: 'crypto-js',
        entry: 'crypto-js',
        globalName: '__crypto_js',
        version: '4.2.0',
        typesPackage: '@types/crypto-js',
        typesVersion: '4.2.2',
        safeModeClass: 'source_bundle',
    },
    {
        id: 'chai',
        entry: 'chai',
        globalName: '__chai',
        version: '6.2.2',
        typesPackage: '@types/chai',
        typesVersion: '5.2.3',
        safeModeClass: 'source_bundle',
    },
    { id: 'ajv', entry: 'ajv', globalName: '__ajv', version: '8.17.1', safeModeClass: 'source_bundle' },
    { id: 'lodash', entry: 'lodash', globalName: '__lodash', version: '4.18.1', safeModeClass: 'source_bundle' },
    // Internal impl dependency of the response visualizer (ADR-202): compiled
    // in-guest at rq.visualizer.set() time. Delivered like any SOURCE_BUNDLE
    // built-in but `internal: true` suppresses the user-facing require() surface.
    // Pinned to 4.7.9 to match packages/variables and the ADR-202 spike.
    {
        id: 'handlebars',
        entry: 'handlebars',
        globalName: '__handlebars',
        version: '4.7.9',
        safeModeClass: 'source_bundle',
        internal: true,
    },
];
/**
 * Pinned @types/node version for editor IntelliSense.
 * All Node built-in modules share this version — bump here to update everywhere.
 */
export const NODE_TYPES_VERSION = '22.15.0';
/**
 * Safe Node.js built-in modules available via require() in user scripts.
 *
 * Two categories:
 * 1. IIFE package dependencies — needed internally by cheerio, csv-parse, uuid, etc.
 * 2. User-facing modules — crypto, buffer, path, etc. for scripting use cases.
 *
 * Security policy: only modules with NO filesystem, network, process, or code
 * execution side effects. See ADR-005 for the full blocked list.
 *
 * Both bare (`require('crypto')`) and node:-prefixed (`require('node:crypto')`)
 * forms are supported — the runtime derives both from this list.
 *
 * Adding a new Node built-in:
 * 1. Verify it has no I/O side effects (see ADR-005 blocked list)
 * 2. Add an entry here — runtime allowlist and editor autocomplete are both derived from it
 */
export const NODE_BUILTIN_PACKAGES = [
    // IIFE package dependencies (existing — needed by cheerio, csv-parse, uuid, etc.)
    // Safe-mode classification (ADR-010 §10/§55):
    //   needs_bridge — reaches a virtualizable host capability satisfied by an
    //     authored data-in/data-out bridge (Buffer, crypto subset, util, stream,
    //     timers/process, zlib).
    //   source_bundle — pure-JS string/URL manipulation with no host capability;
    //     bundled+eval'd in-isolate with no bridge.
    // `events` is served in Safe mode by an in-isolate polyfill IIFE (RQ-5625): the
    // sax parser under xml2js's `parseString` — the path `xml2Json` uses — extends
    // EventEmitter, so a copy-in/copy-out bridge is structurally wrong; it needs a
    // real in-realm EventEmitter. `polyfillEntry: 'events/'` bundles the npm `events`
    // package (Node's events module verbatim), NOT Node's built-in of the same name.
    {
        id: 'events',
        name: 'events',
        description: 'Event emitter',
        safeModeClass: 'source_bundle',
        globalName: '__events',
        polyfillEntry: 'events/',
    },
    { id: 'stream', name: 'stream', description: 'Stream primitives', safeModeClass: 'needs_bridge' },
    {
        id: 'timers',
        name: 'timers',
        description: 'Timer functions (setTimeout, setInterval)',
        safeModeClass: 'needs_bridge',
    },
    {
        id: 'util',
        name: 'util',
        description: 'Utility functions (inspect, format, promisify)',
        safeModeClass: 'needs_bridge',
    },
    // User-facing modules
    { id: 'assert', name: 'assert', description: 'Assertion testing', safeModeClass: 'source_bundle' },
    { id: 'buffer', name: 'buffer', description: 'Binary data manipulation (Buffer)', safeModeClass: 'needs_bridge' },
    {
        id: 'crypto',
        name: 'crypto',
        description: 'Hashing, HMAC, encryption, random bytes',
        safeModeClass: 'needs_bridge',
    },
    { id: 'path', name: 'path', description: 'File path string manipulation', safeModeClass: 'source_bundle' },
    { id: 'punycode', name: 'punycode', description: 'Unicode to ASCII encoding', safeModeClass: 'source_bundle' },
    { id: 'querystring', name: 'querystring', description: 'URL query string parsing', safeModeClass: 'source_bundle' },
    {
        id: 'string_decoder',
        name: 'string_decoder',
        description: 'Buffer to string decoding',
        safeModeClass: 'source_bundle',
    },
    { id: 'url', name: 'url', description: 'URL parsing and formatting', safeModeClass: 'source_bundle' },
    {
        id: 'zlib',
        name: 'zlib',
        description: 'Compression and decompression (gzip, deflate)',
        safeModeClass: 'needs_bridge',
    },
];
