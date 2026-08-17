/**
 * core-globals — minimal web/ES global polyfills for the bare guest realm (ADR-010/012).
 *
 * A QuickJS-WASM context is a BARE ES realm: it has the core ECMAScript globals
 * (`Promise`, `Uint8Array`, `ArrayBuffer`, `Reflect`, `Proxy`, `Symbol`, …) but
 * NONE of the Web/Node host globals — no `TextEncoder`/`TextDecoder`, `atob`/
 * `btoa`, `URL`, `EventTarget`, `queueMicrotask`. SOURCE_BUNDLE packages (Chai
 * references `EventTarget`) and the capability shims (crypto/zlib/fetch use
 * `TextEncoder`) need a baseline. This shim provides that baseline in PURE
 * in-guest JS — no host callback, nothing crosses the edge (containment is
 * trivial). It MUST eval first, before any other shim or bundle.
 *
 * Implementations are minimal-but-correct: UTF-8 encode/decode, base64, a
 * spec-shaped `EventTarget`, and microtask/timer stubs. Anything richer that a
 * package needs and we don't provide is the IMPOSSIBLE tail (Developer mode).
 */
export const CORE_GLOBALS_SHIM = `
(() => {
  // ── TextEncoder / TextDecoder (UTF-8) ──
  if (typeof globalThis.TextEncoder === 'undefined') {
    globalThis.TextEncoder = class TextEncoder {
      get encoding() { return 'utf-8'; }
      encode(str) {
        str = String(str === undefined ? '' : str);
        const out = [];
        for (let i = 0; i < str.length; i++) {
          let c = str.charCodeAt(i);
          if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
            const c2 = str.charCodeAt(i + 1);
            if (c2 >= 0xdc00 && c2 <= 0xdfff) { c = 0x10000 + ((c - 0xd800) << 10) + (c2 - 0xdc00); i++; }
          }
          if (c < 0x80) out.push(c);
          else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
          else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
          else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 0x3f), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
        }
        return new Uint8Array(out);
      }
    };
  }
  if (typeof globalThis.TextDecoder === 'undefined') {
    globalThis.TextDecoder = class TextDecoder {
      constructor(label) { this.encoding = label || 'utf-8'; }
      decode(input) {
        if (!input) return '';
        const bytes = input instanceof Uint8Array ? input : new Uint8Array(input.buffer || input);
        let out = '';
        for (let i = 0; i < bytes.length;) {
          let c = bytes[i++];
          if (c > 0x7f) {
            if (c > 0xdf && c < 0xf0) { c = ((c & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f); }
            else if (c > 0xbf) { c = ((c & 0x1f) << 6) | (bytes[i++] & 0x3f); }
          }
          if (c < 0x10000) out += String.fromCharCode(c);
          else { c -= 0x10000; out += String.fromCharCode(0xd800 + (c >> 10), 0xdc00 + (c & 0x3ff)); }
        }
        return out;
      }
    };
  }

  // ── base64 (atob / btoa) ──
  if (typeof globalThis.btoa === 'undefined') {
    const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    globalThis.btoa = (data) => {
      let str = String(data), out = '';
      for (let i = 0; i < str.length;) {
        const a = str.charCodeAt(i++), b = i < str.length ? str.charCodeAt(i++) : NaN, c = i < str.length ? str.charCodeAt(i++) : NaN;
        const e1 = a >> 2, e2 = ((a & 3) << 4) | (b >> 4), e3 = isNaN(b) ? 64 : ((b & 15) << 2) | (c >> 6), e4 = isNaN(c) ? 64 : c & 63;
        out += B64[e1] + B64[e2] + (e3 === 64 ? '=' : B64[e3]) + (e4 === 64 ? '=' : B64[e4]);
      }
      return out;
    };
    globalThis.atob = (data) => {
      const str = String(data).replace(/=+$/, ''); let out = '';
      const idx = (ch) => B64.indexOf(ch);
      for (let i = 0; i < str.length;) {
        const e1 = idx(str[i++]), e2 = idx(str[i++]), e3 = idx(str[i++]), e4 = idx(str[i++]);
        out += String.fromCharCode((e1 << 2) | (e2 >> 4));
        if (e3 !== -1 && e3 !== undefined) out += String.fromCharCode(((e2 & 15) << 4) | (e3 >> 2));
        if (e4 !== -1 && e4 !== undefined) out += String.fromCharCode(((e3 & 3) << 6) | e4);
      }
      return out;
    };
  }

  // ── Event / CustomEvent (spec-shaped, minimal) — Chai and other SOURCE_BUNDLE
  //    packages reference/subclass them at module top level ──
  if (typeof globalThis.Event === 'undefined') {
    globalThis.Event = class Event {
      constructor(type, init) {
        this.type = String(type);
        this.bubbles = !!(init && init.bubbles);
        this.cancelable = !!(init && init.cancelable);
        this.defaultPrevented = false;
        this.timeStamp = 0;
      }
      preventDefault() { this.defaultPrevented = true; }
      stopPropagation() {}
      stopImmediatePropagation() {}
    };
  }
  if (typeof globalThis.CustomEvent === 'undefined') {
    globalThis.CustomEvent = class CustomEvent extends globalThis.Event {
      constructor(type, init) { super(type, init); this.detail = init ? init.detail : undefined; }
    };
  }

  // ── EventTarget (spec-shaped, minimal) — Chai and other SOURCE_BUNDLE pkgs feature-detect it ──
  if (typeof globalThis.EventTarget === 'undefined') {
    globalThis.EventTarget = class EventTarget {
      constructor() { this.__listeners = {}; }
      addEventListener(type, cb) { (this.__listeners[type] ||= []).push(cb); }
      removeEventListener(type, cb) { this.__listeners[type] = (this.__listeners[type] || []).filter((f) => f !== cb); }
      dispatchEvent(event) { (this.__listeners[event && event.type] || []).slice().forEach((f) => f(event)); return true; }
    };
  }

  // ── microtask + timer stubs (isolate has no event loop primitives) ──
  if (typeof globalThis.queueMicrotask === 'undefined') {
    globalThis.queueMicrotask = (fn) => { Promise.resolve().then(fn); };
  }
  // Real timers, host-driven (RQ-5154, ADR-219). The isolate has no clock, so a
  // timer awaits a host bridge that resolves when the delay elapses. The callback
  // stays in-guest behind an id table — only numbers cross, which is what the
  // copy-in/copy-out invariant requires.
  if (typeof globalThis.setTimeout === 'undefined') {
    const __rqTimers = new Map();
    let __rqNextTimerId = 1;

    // A throw here is a GUEST promise rejection, which the host cannot see
    // (quickjs-emscripten-core 0.32.0 leaves promiseRejectionHandler
    // unimplemented). Catch it in-guest and report it over the bridge, so a
    // broken timer callback is visible rather than silently swallowed.
    const __rqInvoke = (callback, args) => {
      try {
        callback(...args);
      } catch (e) {
        __rq_timerError(e && e.message ? String(e.message) : String(e));
      }
    };

    globalThis.setTimeout = (fn, ms, ...args) => {
      if (typeof fn !== 'function') return 0;
      const id = __rqNextTimerId++;
      __rqTimers.set(id, fn);
      __rq_setTimer(id, ms > 0 ? ms : 0).then(() => {
        const callback = __rqTimers.get(id);
        __rqTimers.delete(id);
        if (callback) __rqInvoke(callback, args);
      });
      return id;
    };

    globalThis.clearTimeout = (id) => {
      __rqTimers.delete(id);
      __rq_clearTimer(id);
    };

    globalThis.setInterval = (fn, ms, ...args) => {
      if (typeof fn !== 'function') return 0;
      const id = __rqNextTimerId++;
      __rqTimers.set(id, fn);
      const tick = () => {
        __rq_setTimer(id, ms > 0 ? ms : 0).then(() => {
          // Cleared while the tick was in flight — do not fire, do not re-arm.
          const callback = __rqTimers.get(id);
          if (!callback) return;
          __rqInvoke(callback, args);
          // Re-arm only if the callback did not clear this interval itself.
          if (__rqTimers.has(id)) tick();
        });
      };
      tick();
      return id;
    };

    globalThis.clearInterval = globalThis.clearTimeout;
    globalThis.setImmediate = (fn, ...args) => globalThis.setTimeout(fn, 0, ...args);
    globalThis.clearImmediate = globalThis.clearTimeout;

    // What require('timers') resolves to (NEEDS_BRIDGE_MODULE_GLOBALS). Built
    // from the SAME functions as the globals above, so the module and the global
    // are one surface and their ids interoperate — the property Developer mode
    // gets by passing its registry wrappers into the require chain (RQ-5671
    // Phase 3). Before this, timers was mapped to __rq_processModule — the
    // process shim — so require('timers').setTimeout was not a function.
    // NOTE: no backticks in this comment; the whole shim is a template literal.
    globalThis.__rq_timersModule = {
      setTimeout: globalThis.setTimeout,
      clearTimeout: globalThis.clearTimeout,
      setInterval: globalThis.setInterval,
      clearInterval: globalThis.clearInterval,
      setImmediate: globalThis.setImmediate,
      clearImmediate: globalThis.clearImmediate,
    };
  }
})();
`;
