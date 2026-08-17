/**
 * In-isolate shim for the Safe-mode `require` bridge.
 *
 * Extracted from `../isolated-require.ts` so it can be imported by a host that has NO Node —
 * the browser host (`@requestly/sandbox-browser`, ADR-204). The shim is a pure
 * string of guest-realm JS with no host dependency, which is precisely what makes
 * "one engine, two hosts" work: BOTH hosts eval this identical text, and only the
 * host callback behind it differs.
 *
 * Keep this file free of imports. `../isolated-require.ts` re-exports it, so every existing
 * import site is unchanged.
 */

export const REQUIRE_ISOLATE_SHIM = `
(() => {
  const resolve = globalThis.__rq_bundleRequire;
  const cache = new Map();
  const evalModule = (code) => {
    const module = { exports: {} };
    const exports = module.exports;
    (function (module, exports, require) {
      eval(code);
    })(module, exports, globalThis.require);
    return module.exports;
  };
  const evalIife = (code, globalName) => {
    // The IIFE is a "var __name = (()=>{...})()" string. Indirect eval runs it in
    // the global scope so the var becomes a property of the isolate globalThis.
    // CRITICAL: strip a leading "use strict"; — in strict mode a var declaration
    // inside eval does NOT create a global property (so e.g. uuid's IIFE, which
    // is emitted with the prefix, would leave __uuid undefined). Mirrors
    // require-builder.ts's host-side handling.
    const src = code.replace(/^"use strict";/, '');
    (0, eval)(src);
    return globalThis[globalName];
  };
  globalThis.require = (id) => {
    if (cache.has(id)) return cache.get(id);
    const res = resolve(id);
    let value;
    if (res.kind === 'bridge') {
      value = globalThis[res.global];
    } else if (res.kind === 'iife') {
      value = evalIife(res.code, res.globalName);
    } else {
      value = evalModule(res.code);
    }
    cache.set(id, value);
    return value;
  };
})();
`;
