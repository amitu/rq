import type { ExternalBuiltinPackageId } from '../../definitions/builtInPackages/index.js';
/**
 * IIFE strings keyed by registry `id` — every EXTERNAL_BUILTIN_PACKAGES entry plus
 * the Node built-ins served by an in-isolate polyfill bundle (e.g. `events`,
 * RQ-5625). The key union is generated from both registries so it cannot drift.
 */
export declare const VENDOR_IIFES: Record<ExternalBuiltinPackageId | 'events', string>;
