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
/**
 * Host timer primitives.
 *
 * This package has no `node` types and no `DOM` lib (ADR-217), so the platform's
 * timers arrive here rather than being imported. The handle never crosses a
 * sandbox boundary — it stays inside the registry.
 *
 * `THandle` is a type parameter rather than `unknown` so a host can wire its own
 * timer type (`NodeJS.Timeout`, `number`, …) with no assertion at the call site.
 * With `unknown`, every host would have to cast in `cancelTimer`, which
 * `gr-no-unsafe-cast` forbids — so the generic is what keeps the rule satisfied
 * by construction rather than by a guard repeated at each wiring site.
 *
 * Deliberately NOT named `setTimer`/`clearTimer`: those are the registry's own
 * public methods, which take a hold and return a registry id. Distinct names
 * keep "the host's raw timer" and "a registered timer" from being confused.
 */
export interface TimerDelegations<THandle = unknown> {
    scheduleTimer(fn: () => void, ms: number): THandle;
    cancelTimer(handle: THandle): void;
}
export interface AsyncRegistryOptions<THandle = unknown> {
    readonly timers: TimerDelegations<THandle>;
    /** Invoked when a registered callback throws synchronously. */
    readonly onCallbackError: (error: unknown) => void;
}
/** Undo a registration. Idempotent — calling it twice decrements once. */
export type SettleFn = () => void;
export declare class AsyncRegistry<THandle = unknown> {
    private readonly options;
    private holding;
    private pending;
    private sealed;
    private nextTimerId;
    private readonly liveTimers;
    constructor(options: AsyncRegistryOptions<THandle>);
    /**
     * Register in-flight work. Returns its (idempotent) settle function.
     *
     * A sealed registry accepts nothing further and hands back a no-op, so work
     * started during teardown cannot resurrect a run that has already ended.
     */
    register(): SettleFn;
    /** Register a promise for the duration of its flight. */
    registerPromise<T>(promise: Promise<T>): Promise<T>;
    /**
     * Schedule a callback. Returns a registry-owned id — never the host handle,
     * which must not escape this class.
     *
     * A synchronous throw in the callback goes to `onCallbackError` rather than
     * escaping into the host's timer machinery, matching `Timerz`'s onError
     * contract (`timers.js:200-201`).
     */
    setTimer(callback: () => void, ms: number): number;
    /** Cancel a scheduled callback and release its hold. Unknown ids are a no-op. */
    clearTimer(id: number): void;
    /** Work that keeps the run alive. The drain waits on this reaching zero. */
    holdingCount(): number;
    /** All in-flight work, holding or not — the seal-and-warn message uses this. */
    pendingCount(): number;
    /**
     * Stop accepting registrations and cancel every live timer.
     *
     * Must run on EVERY exit path, including timeout and throw: a live host timer
     * that outlives the execution fires into a disposed context.
     */
    seal(): void;
}
