/**
 * Keep a `require()`d Node built-in's async work visible to the `AsyncRegistry`
 * (ADR-219, RQ-5671 Phase 3).
 *
 * Developer mode's require chain hands the script the REAL Node module
 * (`require-builder.ts` Tier 5), so anything async it starts is invisible to the
 * drain — `require('timers').setTimeout(cb, 1000)` bypasses the registry-backed
 * global entirely. Safe mode has no such hole: every `needs_bridge` module reaches
 * the host through a counted bridge.
 *
 * Which treatment each built-in gets is declared on its registry entry
 * (`NodeBuiltinPackage.developerAsync`), a REQUIRED field — so a new built-in
 * cannot be added without deciding, which is the standing guarantee that keeps
 * this from being a one-time sweep.
 */
function isFunction(value) {
    return typeof value === 'function';
}
/**
 * Wrap the one-shot callback-style async APIs of a module (`crypto`, `zlib`).
 *
 * Generic by design rather than an enumerated list of function names: enumerating
 * is the failure mode RQ-5671 exists to remove, and Node keeps adding async APIs.
 * The rule is positional — a trailing function argument is a completion callback —
 * which holds across `crypto.randomBytes`, `pbkdf2`, `scrypt`, `generateKeyPair`,
 * `hkdf`, `zlib.gzip`, `brotliCompress`, and anything added later in that shape.
 *
 * Only applied to modules declared `callback-last`. It is deliberately NOT applied
 * to `stream`: there a trailing function is usually an EventEmitter listener
 * (`.on('data', fn)`), which never "completes", so the hold would never release
 * and the run would wait out its whole budget.
 *
 * A sync function that happens to take a trailing callback is harmless — the
 * registration settles as soon as the callback fires, which is immediately.
 */
function wrapCallbackLast(mod, registry) {
    const wrapped = new Map();
    return new Proxy(mod, {
        get(target, prop, receiver) {
            const value = Reflect.get(target, prop, receiver);
            if (!isFunction(value) || typeof prop !== 'string')
                return value;
            const memo = wrapped.get(prop);
            if (memo !== undefined)
                return memo;
            const wrapper = (...args) => {
                const last = args.at(-1);
                if (!isFunction(last))
                    return Reflect.apply(value, target, args);
                const settle = registry.register();
                let settled = false;
                const trackedCallback = (...callbackArgs) => {
                    if (!settled) {
                        settled = true;
                        settle();
                    }
                    return Reflect.apply(last, undefined, callbackArgs);
                };
                try {
                    return Reflect.apply(value, target, [...args.slice(0, -1), trackedCallback]);
                }
                catch (error) {
                    // A synchronous throw means the callback will never fire; release the
                    // hold rather than stranding the run until the deadline.
                    if (!settled) {
                        settled = true;
                        settle();
                    }
                    throw error;
                }
            };
            wrapped.set(prop, wrapper);
            return wrapper;
        },
    });
}
/**
 * Serve the registry's own timer functions in place of Node's, so
 * `require('timers')` and the `setTimeout` global are the same surface.
 *
 * `setImmediate`/`clearImmediate` are provided over `setTimeout(…, 0)` — the
 * `timers` module exposes them, the injected globals do not, and Postman's
 * `Timerz` emulates them the same way when a host lacks them
 * (`timers.js:126-133`).
 */
function registryTimersModule(timers) {
    const setTimeoutFn = timers.setTimeout;
    return {
        ...timers,
        setImmediate: isFunction(setTimeoutFn)
            ? (fn, ...args) => setTimeoutFn(fn, 0, ...args)
            : setTimeoutFn,
        clearImmediate: timers.clearTimeout,
    };
}
/**
 * Apply a built-in's declared Developer async treatment.
 *
 * `not-an-async-source` returns the module untouched — the justification for each
 * such entry lives on the registry entry, not here.
 */
export function applyDeveloperAsyncTreatment(mod, treatment, registry, timers) {
    switch (treatment) {
        case 'registry-timers':
            return registryTimersModule(timers);
        case 'callback-last':
            return typeof mod === 'object' && mod !== null ? wrapCallbackLast(mod, registry) : mod;
        case 'not-an-async-source':
            return mod;
    }
}
