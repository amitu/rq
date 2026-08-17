// Every shim string is guest-realm text and lives in `@requestly/sandbox-engine`
// (ADR-217). Only the ORDER is stated here, and only because it is the Node host
// that owns the canonical one — the browser host asserts parity against it.
import { CONSOLE_ISOLATE_SHIM } from './bridges/console-bridge.js';
import { PROCESS_ISOLATE_SHIM } from './bridges/process-bridge.js';
import { STREAM_ISOLATE_SHIM } from './bridges/stream-bridge.js';
import { BUFFER_ISOLATE_SHIM } from './shims/buffer.shim.js';
import { CRYPTO_ISOLATE_SHIM } from './shims/crypto.shim.js';
import { UTIL_ISOLATE_SHIM } from './shims/util.shim.js';
import { ZLIB_ISOLATE_SHIM } from './shims/zlib.shim.js';
import { FETCH_ISOLATE_SHIM } from './shims/fetch.shim.js';

/**
 * The in-isolate shim strings, eval'd in order inside the isolate after the host
 * callbacks are installed. Console is first (always wired), then the capability
 * shims. crypto must precede any consumer that calls `__rq_concatAB`.
 */
export const ISOLATE_SHIMS: readonly string[] = [
  CONSOLE_ISOLATE_SHIM,
  PROCESS_ISOLATE_SHIM,
  BUFFER_ISOLATE_SHIM,
  CRYPTO_ISOLATE_SHIM,
  UTIL_ISOLATE_SHIM,
  STREAM_ISOLATE_SHIM,
  ZLIB_ISOLATE_SHIM,
  FETCH_ISOLATE_SHIM,
];
