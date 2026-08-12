// ---------------------------------------------------------------------------
// Lazy convenience globals for built-in packages — Postman parity (RQ-5512
// `CryptoJS`, RQ-5613 `_`, RQ-5625 `xml2Json`).
//
// Postman injects some of its bundled libraries as BARE globals, so imported
// Postman scripts reference them with no `require` line at all
// (`postman-sandbox/lib/sandbox/postman-legacy-interface.js` — `CryptoJS`, `_`,
// `xml2Json`, …). A script that says `CryptoJS.HmacSHA256(msg, key)` has to work
// unmodified, with no re-import and no user action.
//
// This is a single source-of-truth string constant eval'd INSIDE each sandbox
// realm at bootstrap — the Developer engine via `vm.runInContext`, the Safe
// (QuickJS) engine via in-guest `evalCode`. Both engines consume this same
// constant so their behavior cannot drift (engine-parity golden rule). This is
// the same pattern as `ARRAY_METHODS_SHIM`, and for the same reason: the two
// engines previously diverged precisely where each hand-rolled its own globals.
//
// Two kinds of global:
//   - DIRECT — the global IS a module (`CryptoJS` = crypto-js, `_` = lodash).
//     Resolves through the realm's own `require()`, so the global and
//     `require('<id>')` return the SAME module instance — one bundle, never two.
//   - WRAPPED — the global is a thin function over a module, not the module
//     itself (`xml2Json` = a wrapper over `require('xml2js').parseString`, matching
//     Postman's `xml2Json`). Produced by a small factory instead of a bare require.
//
// Laziness matters: `require()` of a SOURCE_BUNDLE built-in evaluates that
// package's IIFE. Installing an accessor rather than a value means a script that
// never touches `CryptoJS`/`xml2Json` never pays for the bundle. On first access
// the accessor replaces itself with a plain value, so a hot loop does not re-enter
// the require chain.
//
// Two non-obvious requirements:
//   - NON-ENUMERABLE. An enumerable accessor would be INVOKED by anything that
//     walks the global object (`for..in`, `Object.keys`, the QuickJS collect
//     path), forcing the bundle eval and defeating the laziness above. Bare
//     identifier resolution does not depend on enumerability.
//   - A SETTER is required. A getter-only accessor makes `CryptoJS = x` throw a
//     TypeError in strict mode and breaks a top-level `var CryptoJS = ...`, so a
//     user script that assigns over the global would fail. The setter keeps the
//     same ergonomics as the plain writable globals it sits alongside.
//
// NOTE ON SCOPE: `_` (RQ-5613) and `xml2Json` (RQ-5625) both moved here out of the
// eager Developer-only `buildConvenienceGlobals()` so BOTH engines install them
// from this one source. `xml2Json` in Safe depends on `require('events')` working
// in the isolate — the sax parser under xml2js's `parseString` extends EventEmitter
// — which RQ-5625 enabled via the `events` polyfill IIFE (`generate-vendor-iifes.ts`,
// registry `events.globalName`/`polyfillEntry`). `cheerio` is intentionally NOT
// here: Postman does not inject it as a bare global, it is require-only.
// ---------------------------------------------------------------------------

export const CONVENIENCE_GLOBALS_SHIM = `
(() => {
  // Install a lazy, non-enumerable accessor \`name\` whose value is \`produce()\`,
  // computed on first read then frozen in as a plain writable value.
  const installLazyGlobal = (name, produce) => {
    // Never clobber something the realm already defined.
    if (Object.prototype.hasOwnProperty.call(globalThis, name)) return;

    // Replace the accessor with a plain writable value, so later reads and
    // writes are ordinary property access with no require-chain round trip.
    const settle = (value) => {
      Object.defineProperty(globalThis, name, {
        configurable: true,
        enumerable: false,
        writable: true,
        value: value,
      });
      return value;
    };

    // Re-entrancy guard (required by lodash's \`_\`, harmless for the others): a
    // package's own UMD bootstrap may READ its bare global while it is still
    // initializing — lodash does \`var oldDash = root._\` for noConflict. The
    // require cache is populated only AFTER the bundle finishes eval'ing, so a
    // naive getter would call \`produce()\` again mid-eval and recurse into the same
    // bundle until the stack overflows (this also breaks the internal
    // \`require('lodash')\` in rq.expect().jsonBody()). While \`produce\` is in flight
    // we return \`undefined\` — exactly what a bare realm's global is before the
    // package exists, matching what the Developer engine sees during that same
    // package's IIFE.
    let loading = false;
    const load = () => {
      if (loading) return undefined;
      loading = true;
      try {
        return settle(produce());
      } finally {
        loading = false;
      }
    };

    Object.defineProperty(globalThis, name, {
      configurable: true,
      enumerable: false,
      get: load,
      set: (value) => { settle(value); },
    });
  };

  // DIRECT globals — bare name → the require id it resolves to.
  const LAZY_BUILTIN_GLOBALS = { CryptoJS: 'crypto-js', _: 'lodash' };
  for (const name of Object.keys(LAZY_BUILTIN_GLOBALS)) {
    const moduleId = LAZY_BUILTIN_GLOBALS[name];
    installLazyGlobal(name, () => globalThis.require(moduleId));
  }

  // WRAPPED global — \`xml2Json\` is Postman's thin wrapper over xml2js.parseString,
  // NOT the xml2js module. Behaviour-identical to Developer mode's former eager
  // injection (buildConvenienceGlobals): explicitArray:false / async:false /
  // trim:true / mergeAttrs:false, throw-on-error, return the parsed object. Kept in
  // sync across engines by this shared source + the Dev/Safe parity test.
  installLazyGlobal('xml2Json', () => {
    const parseString = globalThis.require('xml2js').parseString;
    return function xml2Json(xmlString) {
      let result;
      let error;
      parseString(xmlString, {
        explicitArray: false,
        async: false,
        trim: true,
        mergeAttrs: false,
      }, function (err, parsed) {
        if (err) { error = err; }
        else { result = parsed; }
      });
      if (error) { throw error; }
      return result;
    };
  });
})();
`;
