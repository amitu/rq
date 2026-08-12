/**
 * Canonical list of all runtime-injected globals.
 *
 * These are injected into vm.createContext() by the builder.
 * The CLI uses this list to extract types from lib.webworker.d.ts.
 *
 * Note: EventTarget/Event are included because Chai IIFE requires them at runtime.
 *
 * Excluded:
 * - ReadableStream/WritableStream/TransformStream: internal use only
 * - importScripts: no vm.createContext() support
 */
export const GLOBAL_NAMES = [
    'console',
    'setTimeout',
    'setInterval',
    'clearTimeout',
    'clearInterval',
    'EventTarget',
    'Event',
    'fetch',
    'URL',
    'URLSearchParams',
    'Headers',
    'Request',
    'Response',
    'AbortController',
    'AbortSignal',
    'TextEncoder',
    'TextDecoder',
    'crypto',
    'Blob',
    'FormData',
    'structuredClone',
    'atob',
    'btoa',
    'performance',
];
/**
 * Extended list for the CLI type extraction.
 *
 * Includes GLOBAL_NAMES + related interface/type names needed for
 * complete type declarations in the editor (e.g. RequestInit, Console, etc.).
 * Transitive dependencies are resolved automatically by the CLI.
 */
export const EDITOR_TYPE_NAMES = [
    ...GLOBAL_NAMES,
    // Interface names for globals (not injected, but needed for type completeness)
    'Console',
    'RequestInit',
    'HeadersInit',
    'Body',
    'RequestInfo',
    'RequestRedirect',
    'RequestCredentials',
    'ReferrerPolicy',
    'ResponseType',
    'RequestMode',
    'RequestCache',
    'RequestDestination',
    'TextEncoderCommon',
    'TextDecoderCommon',
    'TextDecoderStream',
    'TextEncoderStream',
    'Crypto',
    'BlobPropertyBag',
    'XMLHttpRequest',
    'XMLHttpRequestEventTarget',
    'XMLHttpRequestUpload',
    'XMLHttpRequestBodyInit',
    'XMLHttpRequestResponseType',
    'ProgressEvent',
    'Performance',
    'PerformanceEntry',
    'PerformanceMark',
    'PerformanceMeasure',
    'PerformanceMarkOptions',
    'PerformanceMeasureOptions',
];
