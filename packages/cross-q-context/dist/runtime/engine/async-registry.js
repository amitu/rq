/**
 * The single owner of "what counts as pending work" for both sandbox engines
 * (RQ-5671 / RQ-5154, ADR-219).
 *
 * RQ-5156 shipped two mechanisms for the same job: Developer counts with a
 * closure counter fed by hand-placed `track()` calls, Safe counts inside the
 * bridge factory. Coverage in Safe is a property of construction; in Developer
 * it is a property of someone having remembered. This registry replaces both,
 * so an async surface is either registered or it is a compile error.
 *
 * Modelled on `postman-sandbox`'s `Timerz` (`lib/sandbox/timers.js`): every
 * registration holds the run open until it settles, intervals included.
 */
export class AsyncRegistry {
    options;
    holding = 0;
    pending = 0;
    sealed = false;
    nextTimerId = 1;
    liveTimers = new Map();
    constructor(options) {
        this.options = options;
    }
    /**
     * Register in-flight work. Returns its (idempotent) settle function.
     *
     * A sealed registry accepts nothing further and hands back a no-op, so work
     * started during teardown cannot resurrect a run that has already ended.
     */
    register() {
        if (this.sealed)
            return () => { };
        this.pending += 1;
        this.holding += 1;
        let settled = false;
        return () => {
            if (settled)
                return;
            settled = true;
            this.pending -= 1;
            this.holding -= 1;
        };
    }
    /** Register a promise for the duration of its flight. */
    registerPromise(promise) {
        const settle = this.register();
        // `.finally` returns a DERIVED promise, which is what the caller receives.
        // An unhandled rejection is therefore still reported exactly once with the
        // original reason: we neither swallow a real signal (as an `onRejected`
        // handler would) nor fabricate one for a rejection the caller handled.
        return promise.finally(settle);
    }
    /**
     * Schedule a callback. Returns a registry-owned id — never the host handle,
     * which must not escape this class.
     *
     * A synchronous throw in the callback goes to `onCallbackError` rather than
     * escaping into the host's timer machinery, matching `Timerz`'s onError
     * contract (`timers.js:200-201`).
     */
    setTimer(callback, ms) {
        if (this.sealed)
            return 0;
        const id = this.nextTimerId++;
        const settle = this.register();
        const handle = this.options.timers.scheduleTimer(() => {
            this.liveTimers.delete(id);
            // Settle BEFORE invoking, for two reasons. A callback that inspects the
            // registry (or schedules more work) must not see its own already-fired
            // timer as still pending, or a drain check made from inside a callback is
            // off by one and seal-and-warn over-reports. And should `onCallbackError`
            // itself throw, the hold is already released rather than stranded for the
            // rest of the execution budget.
            settle();
            try {
                callback();
            }
            catch (error) {
                this.options.onCallbackError(error);
            }
        }, ms);
        // The settle fn is stored with the handle so `clearTimer` can release the
        // hold without re-deriving it.
        this.liveTimers.set(id, { handle, settle });
        return id;
    }
    /** Cancel a scheduled callback and release its hold. Unknown ids are a no-op. */
    clearTimer(id) {
        const entry = this.liveTimers.get(id);
        if (entry === undefined)
            return;
        this.liveTimers.delete(id);
        this.options.timers.cancelTimer(entry.handle);
        entry.settle();
    }
    /** Work that keeps the run alive. The drain waits on this reaching zero. */
    holdingCount() {
        return this.holding;
    }
    /** All in-flight work, holding or not — the seal-and-warn message uses this. */
    pendingCount() {
        return this.pending;
    }
    /**
     * Stop accepting registrations and cancel every live timer.
     *
     * Must run on EVERY exit path, including timeout and throw: a live host timer
     * that outlives the execution fires into a disposed context.
     */
    seal() {
        this.sealed = true;
        for (const entry of this.liveTimers.values()) {
            this.options.timers.cancelTimer(entry.handle);
            entry.settle();
        }
        this.liveTimers.clear();
    }
}
