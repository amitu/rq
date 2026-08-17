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

import { createIgnoredBridge, createSafeBridge } from '../safe-bridge-factory.js';

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
export function createTimerBridges<THandle>(
  registry: AsyncRegistry<THandle>,
  onGuestCallbackError: (message: string) => void,
): readonly SafeBridge[] {
  /**
   * Guest timer id → its registry id and the resolver of its in-flight arm.
   *
   * The resolver is held because a cancelled registry timer never invokes its
   * callback, so nothing would settle the guest promise — the async bridge's
   * in-flight count would never decrement and the pump loop would spin until the
   * execution deadline. `clearTimeout` must therefore settle it explicitly.
   */
  const live = new Map<number, { registryId: number; settle: () => void }>();

  const arm = (guestId: number, ms: number): Promise<undefined> =>
    new Promise<undefined>((resolve) => {
      const settle = (): void => {
        resolve(undefined);
      };
      const registryId = registry.setTimer(() => {
        live.delete(guestId);
        settle();
      }, ms);
      live.set(guestId, { registryId, settle });
    });

  return [
    createSafeBridge('__rq_setTimer', (guestId: number, ms: number) => arm(guestId, ms), {
      async: true,
    }),
    createSafeBridge('__rq_clearTimer', (guestId: number) => {
      const entry = live.get(guestId);
      if (entry !== undefined) {
        live.delete(guestId);
        registry.clearTimer(entry.registryId);
        // Settle the guest promise the cancelled timer would otherwise strand.
        // The guest has already dropped its callback from the id table, so the
        // resulting `.then` is a no-op — this exists purely to release the count.
        entry.settle();
      }
      return undefined;
    }),
    // Fire-and-forget: a throw inside a guest timer callback is a GUEST promise
    // rejection, which QuickJS cannot surface (`promiseRejectionHandler` is
    // unimplemented in quickjs-emscripten-core 0.32.0 — ADR-219's recorded
    // non-goal). The shim therefore catches it in-guest and reports it here, so
    // the engines still agree on callback-throw visibility.
    createIgnoredBridge('__rq_timerError', (message: string) => {
      onGuestCallbackError(message);
    }),
  ];
}
