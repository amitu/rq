import type { StreamReadResult, StreamReader } from '../contract.js';

/**
 * A generic, push-based event stream buffer that implements StreamReader<T>.
 *
 * Transport-agnostic — no RPC or platform dependency. Consumers that need
 * RPC proxying (e.g., desktop via capnweb) inject the transport brand via
 * prototype injection (see desktop ADR-016).
 *
 * Producer (local, NOT part of StreamReader interface):
 *   push(event)       — enqueue an event
 *   end()             — signal no more events
 *   fail(err)         — signal an error
 *   checkCancelled()  — cooperative cancellation check
 *   onCancel(cb)      — register a callback invoked when cancel() fires
 *
 * Consumer (implements StreamReader<T>):
 *   read()   — returns next event or { done: true }
 *   cancel() — signal cancellation
 *
 * Single-consumer only — concurrent read() calls throw.
 *
 * Note: push()/end() are no-ops inside onCancel callbacks because cancel()
 * sets the terminal flags before firing the callback. If a producer needs to
 * push a final event on cancellation, it should do so before calling end(),
 * not from within the onCancel handler.
 *
 * @see ADR-036 for behavioral choices and extraction rationale.
 */
export class StreamHandle<T> implements StreamReader<T> {
  private buffer: T[] = [];
  private waiter: {
    resolve: (result: StreamReadResult<T>) => void;
    reject: (err: unknown) => void;
  } | null = null;
  private ended = false;
  private error: unknown = null;
  private cancelled = false;
  private onCancelCallback: (() => void) | null = null;

  /** Check if the consumer has cancelled. Producers should check this between async operations. */
  checkCancelled(): boolean {
    return this.cancelled;
  }

  /** Number of buffered events waiting to be read by the consumer. */
  get bufferSize(): number {
    return this.buffer.length;
  }

  /**
   * Register a callback invoked synchronously when cancel() is called.
   * Producer-only — NOT part of StreamReader interface.
   *
   * If cancel() has already been called, the callback fires immediately
   * (retroactive-fire pattern — prevents race conditions where the producer
   * registers onCancel after the consumer has already cancelled).
   *
   * Single-registration: calling again replaces the previous callback.
   */
  onCancel(callback: () => void): void {
    this.onCancelCallback = callback;
    if (this.cancelled) {
      callback();
    }
  }

  // ─── Producer API (called locally by the service) ───────────────────

  /** Enqueue an event. Delivers to a waiting consumer or buffers. */
  push(event: T): void {
    if (this.ended || this.cancelled) return;

    if (this.waiter) {
      const { resolve } = this.waiter;
      this.waiter = null;
      resolve({ done: false, value: event });
    } else {
      this.buffer.push(event);
    }
  }

  /** Signal that no more events will be produced. */
  end(): void {
    if (this.ended) return;
    this.ended = true;

    if (this.waiter) {
      const { resolve } = this.waiter;
      this.waiter = null;
      resolve({ done: true });
    }
  }

  /** Signal an error. Discards buffered items and wakes any pending reader by rejecting with the error. */
  fail(err: unknown): void {
    if (this.ended) return;
    this.ended = true;
    this.error = err;
    this.buffer.length = 0;

    if (this.waiter) {
      const { reject } = this.waiter;
      this.waiter = null;
      reject(err instanceof Error ? err : new Error(String(err)));
    }
  }

  // ─── Consumer API (implements StreamReader<T>) ──────────────────────

  /**
   * Read the next event from the stream.
   *
   * - If buffer has data, resolves immediately.
   * - If buffer is empty and stream is ended, returns { done: true }.
   * - If buffer is empty and stream has failed, rejects with the stored error.
   * - If buffer is empty and stream is active, waits for next push().
   *
   * Throws if called while a previous read() is still pending (single-consumer).
   */
  read(): Promise<StreamReadResult<T>> {
    if (this.cancelled) {
      return Promise.resolve({ done: true });
    }

    const next = this.buffer.shift();
    if (next !== undefined) {
      return Promise.resolve({ done: false, value: next });
    }

    if (this.ended) {
      if (this.error) {
        return Promise.reject(
          this.error instanceof Error
            ? this.error
            : new Error(typeof this.error === 'string' ? this.error : JSON.stringify(this.error)),
        );
      }
      return Promise.resolve({ done: true });
    }

    if (this.waiter !== null) {
      throw new Error('StreamHandle: concurrent read() calls are not supported');
    }

    return new Promise((resolve, reject) => {
      this.waiter = { resolve, reject };
    });
  }

  /**
   * Cancel the stream. Sets cancelled + ended, clears buffer,
   * and wakes any pending read() with { done: true }.
   */
  cancel(): Promise<void> {
    if (this.cancelled) {
      return Promise.resolve();
    }

    this.cancelled = true;
    this.ended = true;
    this.buffer.length = 0;

    if (this.waiter) {
      const { resolve } = this.waiter;
      this.waiter = null;
      resolve({ done: true });
    }

    this.onCancelCallback?.();

    return Promise.resolve();
  }
}
