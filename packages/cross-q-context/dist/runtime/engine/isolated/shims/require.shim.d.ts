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
export declare const REQUIRE_ISOLATE_SHIM = "\n(() => {\n  const resolve = globalThis.__rq_bundleRequire;\n  const cache = new Map();\n  const evalModule = (code) => {\n    const module = { exports: {} };\n    const exports = module.exports;\n    (function (module, exports, require) {\n      eval(code);\n    })(module, exports, globalThis.require);\n    return module.exports;\n  };\n  const evalIife = (code, globalName) => {\n    // The IIFE is a \"var __name = (()=>{...})()\" string. Indirect eval runs it in\n    // the global scope so the var becomes a property of the isolate globalThis.\n    // CRITICAL: strip a leading \"use strict\"; \u2014 in strict mode a var declaration\n    // inside eval does NOT create a global property (so e.g. uuid's IIFE, which\n    // is emitted with the prefix, would leave __uuid undefined). Mirrors\n    // require-builder.ts's host-side handling.\n    const src = code.replace(/^\"use strict\";/, '');\n    (0, eval)(src);\n    return globalThis[globalName];\n  };\n  globalThis.require = (id) => {\n    if (cache.has(id)) return cache.get(id);\n    const res = resolve(id);\n    let value;\n    if (res.kind === 'bridge') {\n      value = globalThis[res.global];\n    } else if (res.kind === 'iife') {\n      value = evalIife(res.code, res.globalName);\n    } else {\n      value = evalModule(res.code);\n    }\n    cache.set(id, value);\n    return value;\n  };\n})();\n";
