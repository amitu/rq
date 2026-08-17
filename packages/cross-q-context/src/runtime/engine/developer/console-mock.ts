import { LogLevel } from '../../index.js';

import { normalizeFromVm } from './realm-normalization.js';

import type { LogEntry } from '../../index.js';

/**
 * Creates a mock console object for the VM context.
 * Each console method (log, warn, error, info) calls the provided callback
 * with a normalized LogEntry. Args are realm-normalized via JSON roundtrip.
 */
export function createConsoleMock(onLog: (log: LogEntry) => void): {
  log: (...args: unknown[]) => void;
  warn: (...args: unknown[]) => void;
  error: (...args: unknown[]) => void;
  info: (...args: unknown[]) => void;
} {
  const pushLog = (level: LogLevel, args: unknown[]): void => {
    const normalized = normalizeFromVm(args);
    const normalizedArgs: unknown[] = Array.isArray(normalized) ? normalized : args.map((a) => String(a));

    const log: LogEntry = {
      level,
      // Normalized console args are JSON-serialized out of the VM realm — Json by construction.
      args: normalizedArgs as LogEntry['args'],
      timestamp: Date.now(),
    };
    onLog(log);
  };

  return {
    log: (...args: unknown[]) => pushLog(LogLevel.log, args),
    warn: (...args: unknown[]) => pushLog(LogLevel.warn, args),
    error: (...args: unknown[]) => pushLog(LogLevel.error, args),
    info: (...args: unknown[]) => pushLog(LogLevel.info, args),
  };
}
