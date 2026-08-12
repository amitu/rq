// ---------------------------------------------------------------------------
// Array method shim — Postman parity (ADR-192, Slice 2).
//
// Postman augments `Array.prototype` (via vendored SugarJS) so user scripts can
// call `arr.first()` / `arr.last()`. Requestly ships ONLY these two methods (the
// customer corpus uses no other SugarJS array methods; everything else is native
// JS and works once `get` returns a real array).
//
// This is a single source-of-truth string constant eval'd INSIDE each sandbox
// realm at bootstrap — the Developer engine via `vm.runInContext`, the Safe
// (QuickJS) engine via in-guest `evalCode`. It must NEVER be run against the host
// `Array.prototype` (that would leak to the renderer). Both engines consume this
// same constant so their behavior cannot drift (engine-parity golden rule).
//
// The methods are defined non-enumerable so they never appear in `for..in` /
// `Object.keys` and cannot corrupt the QuickJS JSON.stringify collect path.
// Semantics match Postman/SugarJS exactly:
//   [1,2,3].first()  -> 1        [1,2,3].first(2) -> [1,2]
//   [1,2,3].last()   -> 3        [1,2,3].last(2)  -> [2,3]
// ---------------------------------------------------------------------------
export const ARRAY_METHODS_SHIM = `
(function () {
  if (typeof Array.prototype.first !== 'function') {
    Object.defineProperty(Array.prototype, 'first', {
      value: function (num) {
        return num === undefined ? this[0] : this.slice(0, num < 0 ? 0 : num);
      },
      enumerable: false,
      writable: true,
      configurable: true,
    });
  }
  if (typeof Array.prototype.last !== 'function') {
    Object.defineProperty(Array.prototype, 'last', {
      value: function (num) {
        if (num === undefined) return this[this.length - 1];
        var start = this.length - num < 0 ? 0 : this.length - num;
        return this.slice(start);
      },
      enumerable: false,
      writable: true,
      configurable: true,
    });
  }
  // Realm reviver (Developer engine only — ADR-192). \`rq.*.get()\` runs its
  // JSON.parse in the HOST realm, so a returned array carries the host
  // Array.prototype, not this realm's patched one. The Developer engine wraps
  // get() to route array results through this reviver, passing the JSON string
  // so the parse — and thus the resulting array — happens in THIS realm and
  // inherits .first()/.last(). (structuredClone is not defined inside a
  // node:vm realm, so JSON round-trip is the portable in-realm rebuild; nested
  // arrays are rebuilt in-realm too.) The Safe (QuickJS) engine needs no
  // equivalent — its parse already happens in-guest.
  if (typeof globalThis.__rq_reviveArrayInRealm !== 'function') {
    Object.defineProperty(globalThis, '__rq_reviveArrayInRealm', {
      value: function (json) {
        return JSON.parse(json);
      },
      enumerable: false,
      writable: true,
      configurable: true,
    });
  }
})();
`;
