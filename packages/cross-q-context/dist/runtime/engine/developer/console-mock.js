import { LogLevel } from '../../index.js';
import { normalizeFromVm } from './realm-normalization.js';
/**
 * Creates a mock console object for the VM context.
 * Each console method (log, warn, error, info) calls the provided callback
 * with a normalized LogEntry. Args are realm-normalized via JSON roundtrip.
 */
export function createConsoleMock(onLog) {
    const pushLog = (level, args) => {
        const normalized = normalizeFromVm(args);
        const normalizedArgs = Array.isArray(normalized) ? normalized : args.map((a) => String(a));
        const log = {
            level,
            // Normalized console args are JSON-serialized out of the VM realm — Json by construction.
            args: normalizedArgs,
            timestamp: Date.now(),
        };
        onLog(log);
    };
    return {
        log: (...args) => pushLog(LogLevel.log, args),
        warn: (...args) => pushLog(LogLevel.warn, args),
        error: (...args) => pushLog(LogLevel.error, args),
        info: (...args) => pushLog(LogLevel.info, args),
    };
}
