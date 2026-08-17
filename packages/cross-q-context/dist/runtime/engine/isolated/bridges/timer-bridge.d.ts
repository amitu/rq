/**
 * Guest timer support (RQ-5154, ADR-219).
 *
 * The isolate has no clock, so timers must be host-driven. Callbacks never cross
 * the boundary — `createSafeBridge`'s `Copyable` constraint makes that a compile
 * error — so the guest keeps an id→function table and crosses with NUMBERS only:
 * it awaits `__rq_setTimer(id, ms)`, and invokes its own callback when the
 * returned promise resolves. Nothing host-side is reachable through any of this,
 * which is what keeps RQ-2489 closed on the new surface.
 *
 * One arming bridge: every timer holds the run open, intervals included. Measured
 * against Postman 12.14.0 — an uncleared `setInterval` there holds the run
 * INDEFINITELY (5040+ ticks / 42 min observed, no termination), because the app
 * passes no finite timeout to the sandbox so `Timerz`'s guard timer is never
 * armed. Holding matches that; the per-execution budget then bounds the runaway
 * case Postman leaves unbounded, and seal-and-warn keeps RQ-5156 intact.
 */
import type { AsyncRegistry } from '../../async-registry.js';
import type { SafeBridge } from '../safe-bridge-factory.js';
/**
 * Build the timer bridges over a per-execution registry.
 *
 * The guest→timer map lives in this closure, so it is per execution like the
 * registry itself: two concurrent runs in one worker never observe each other's
 * timers.
 *
 * @param registry per-execution async registry
 * @param onGuestCallbackError reports a throw from inside a guest timer callback
 */
export declare function createTimerBridges<THandle>(registry: AsyncRegistry<THandle>, onGuestCallbackError: (message: string) => void): readonly SafeBridge[];
