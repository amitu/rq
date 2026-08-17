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
export declare class StreamHandle<T> implements StreamReader<T> {
    private buffer;
    private waiter;
    private ended;
    private error;
    private cancelled;
    private onCancelCallback;
    /** Check if the consumer has cancelled. Producers should check this between async operations. */
    checkCancelled(): boolean;
    /** Number of buffered events waiting to be read by the consumer. */
    get bufferSize(): number;
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
    onCancel(callback: () => void): void;
    /** Enqueue an event. Delivers to a waiting consumer or buffers. */
    push(event: T): void;
    /** Signal that no more events will be produced. */
    end(): void;
    /** Signal an error. Discards buffered items and wakes any pending reader by rejecting with the error. */
    fail(err: unknown): void;
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
    read(): Promise<StreamReadResult<T>>;
    /**
     * Cancel the stream. Sets cancelled + ended, clears buffer,
     * and wakes any pending read() with { done: true }.
     */
    cancel(): Promise<void>;
}
