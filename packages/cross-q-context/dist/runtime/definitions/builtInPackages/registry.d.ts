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
export declare const EXTERNAL_BUILTIN_PACKAGES: readonly [{
    readonly id: "moment";
    readonly entry: "moment";
    readonly globalName: "__moment";
    readonly version: "2.30.1";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "xml2js";
    readonly entry: "xml2js";
    readonly globalName: "__xml2js";
    readonly version: "0.6.2";
    readonly typesPackage: "@types/xml2js";
    readonly typesVersion: "0.4.14";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "uuid";
    readonly entry: "uuid";
    readonly globalName: "__uuid";
    readonly version: "11.1.0";
    readonly typesPackage: "@types/uuid";
    readonly typesVersion: "10.0.0";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "csv-parse/lib/sync";
    readonly entry: "csv-parse/sync";
    readonly globalName: "__csv_parse";
    readonly version: "5.6.0";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "cheerio";
    readonly entry: "cheerio";
    readonly globalName: "__cheerio";
    readonly version: "1.0.0";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "crypto-js";
    readonly entry: "crypto-js";
    readonly globalName: "__crypto_js";
    readonly version: "4.2.0";
    readonly typesPackage: "@types/crypto-js";
    readonly typesVersion: "4.2.2";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "chai";
    readonly entry: "chai";
    readonly globalName: "__chai";
    readonly version: "6.2.2";
    readonly typesPackage: "@types/chai";
    readonly typesVersion: "5.2.3";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "ajv";
    readonly entry: "ajv";
    readonly globalName: "__ajv";
    readonly version: "8.17.1";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "lodash";
    readonly entry: "lodash";
    readonly globalName: "__lodash";
    readonly version: "4.18.1";
    readonly safeModeClass: "source_bundle";
}, {
    readonly id: "handlebars";
    readonly entry: "handlebars";
    readonly globalName: "__handlebars";
    readonly version: "4.7.9";
    readonly safeModeClass: "source_bundle";
    readonly internal: true;
}];
export type ExternalBuiltinPackageId = (typeof EXTERNAL_BUILTIN_PACKAGES)[number]['id'];
/**
 * Pinned @types/node version for editor IntelliSense.
 * All Node built-in modules share this version — bump here to update everywhere.
 */
export declare const NODE_TYPES_VERSION = "22.15.0";
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
export declare const NODE_BUILTIN_PACKAGES: readonly [{
    readonly id: "events";
    readonly name: "events";
    readonly description: "Event emitter";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
    readonly globalName: "__events";
    readonly polyfillEntry: "events/";
}, {
    readonly id: "stream";
    readonly name: "stream";
    readonly description: "Stream primitives";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "timers";
    readonly name: "timers";
    readonly description: "Timer functions (setTimeout, setInterval)";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "registry-timers";
}, {
    readonly id: "util";
    readonly name: "util";
    readonly description: "Utility functions (inspect, format, promisify)";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "assert";
    readonly name: "assert";
    readonly description: "Assertion testing";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "buffer";
    readonly name: "buffer";
    readonly description: "Binary data manipulation (Buffer)";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "crypto";
    readonly name: "crypto";
    readonly description: "Hashing, HMAC, encryption, random bytes";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "callback-last";
}, {
    readonly id: "path";
    readonly name: "path";
    readonly description: "File path string manipulation";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "punycode";
    readonly name: "punycode";
    readonly description: "Unicode to ASCII encoding";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "querystring";
    readonly name: "querystring";
    readonly description: "URL query string parsing";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "string_decoder";
    readonly name: "string_decoder";
    readonly description: "Buffer to string decoding";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "url";
    readonly name: "url";
    readonly description: "URL parsing and formatting";
    readonly safeModeClass: "source_bundle";
    readonly developerAsync: "not-an-async-source";
}, {
    readonly id: "zlib";
    readonly name: "zlib";
    readonly description: "Compression and decompression (gzip, deflate)";
    readonly safeModeClass: "needs_bridge";
    readonly developerAsync: "callback-last";
}];
export type NodeBuiltinPackageId = (typeof NODE_BUILTIN_PACKAGES)[number]['id'];
