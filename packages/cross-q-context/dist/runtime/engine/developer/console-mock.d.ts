import type { LogEntry } from '../../index.js';
/**
 * Creates a mock console object for the VM context.
 * Each console method (log, warn, error, info) calls the provided callback
 * with a normalized LogEntry. Args are realm-normalized via JSON roundtrip.
 */
export declare function createConsoleMock(onLog: (log: LogEntry) => void): {
    log: (...args: unknown[]) => void;
    warn: (...args: unknown[]) => void;
    error: (...args: unknown[]) => void;
    info: (...args: unknown[]) => void;
};
