import type { SafeBridge } from './isolated/safe-bridge-factory.js';
import type { SendRequestFn } from './host-types.js';
/** Build the `__rq_fetch` async bridge that delegates each request to `sendRequest`. */
export declare function createFetchBridge(sendRequest: SendRequestFn): SafeBridge;
