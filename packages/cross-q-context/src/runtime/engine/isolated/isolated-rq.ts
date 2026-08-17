/**
 * isolated-rq — the in-isolate `rq.*` namespace + Chai (ADR-010 SCOPE, Slice 2).
 *
 * Builds the rq namespace INSIDE the QuickJS WASM realm from copied context data.
 * The host `createRqNamespace` (sandbox-definitions) produces closure-bearing objects
 * that cannot cross the isolate edge per the HARD INVARIANT; this shim rebuilds an
 * equivalent surface using only copied data + host bridge callbacks.
 *
 * PARITY CONTRACT: every member of the Developer-mode rq namespace must have a
 * corresponding implementation here. The parity test
 * (../__tests__/rq-namespace-parity.test.ts) enumerates both and flags divergence.
 * When adding a new rq.* member to sandbox-definitions/rqMethods.ts, add the
 * Safe-mode equivalent here — the parity test will catch omissions.
 */

import type { RawScopeMutations, TestResult, ExecutionDirective, RequestHeaderMutation } from '../../index.js';
import type { VisualizerDirective } from '../../definitions/_deps.js';

/**
 * In-isolate JS: builds `globalThis.rq` over the copied `__rq_context` and the
 * in-isolate Chai (`__rq_chai`). Collects test results and mutations on isolate
 * globals the engine drains afterward.
 */
export const RQ_ISOLATE_SHIM = `
(() => {
  const ctx = globalThis.__rq_context || {};
  const chai = globalThis.__rq_chai;
  const testResults = [];
  const mutations = { global: {}, environment: {}, collection: {}, runtime: {} };
  const requestMutations = [];
  globalThis.__rq_testResults = testResults;
  globalThis.__rq_mutations = mutations;
  globalThis.__rq_requestMutations = requestMutations;
  // Single flow-control directive (ADR-169) — not an array; the last write wins,
  // mirroring the Developer collector's single \`directive\` field.
  globalThis.__rq_executionDirective = undefined;
  // Single visualizer intent (ADR-202) — available in BOTH phases (Postman parity,
  // "Amendment (2026-08-02)"); last set() wins, clear() records a { kind: 'cleared' }
  // marker (distinct from this initial "no call" absent, FR-18c). Drained via
  // RQ_COLLECT_EXPR after the script settles, regardless of phase.
  globalThis.__rq_visualizerOutput = undefined;

  const test = (name, fn) => {
    if (typeof fn !== 'function') { testResults.push({ status: 'skipped', name: String(name) }); return; }
    try { fn(); testResults.push({ status: 'passed', name: String(name) }); }
    catch (err) { testResults.push({ status: 'failed', name: String(name), error: err && err.message ? String(err.message) : String(err) }); }
  };
  test.skip = (name) => { testResults.push({ status: 'skipped', name: String(name) }); };

  // ── Type coercion (RQ-1421 parity) ──
  // When a script sets a number/boolean, reads back the same type, not a string.
  const coerceByType = (value, type) => {
    if (type === 'number') {
      if (value === '') return value;
      const n = Number(value);
      return Number.isFinite(n) ? n : value;
    }
    if (type === 'boolean') return value === 'true';
    if (type === 'array') {
      // Arrays are stored JSON-encoded (ADR-192); parse back to a real array.
      // Guard failure / non-array by returning the raw string (mirrors Developer
      // engine coerceValueByType — engine-behavior parity).
      try {
        const parsed = JSON.parse(value);
        return Array.isArray(parsed) ? parsed : value;
      } catch {
        return value;
      }
    }
    return value;
  };
  const rawType = (v) => {
    if (Array.isArray(v)) return 'array';
    return typeof v === 'number' ? 'number' : typeof v === 'boolean' ? 'boolean' : 'string';
  };
  // Arrays JSON-encode (ADR-192); scalars stringify. Mirrors the Developer engine \`set\`.
  const rawEntry = (v) =>
    Array.isArray(v) ? { value: JSON.stringify(v), type: 'array' } : { value: typeof v === 'string' ? v : String(v), type: rawType(v) };

  const effective = (vd) => {
    if (vd == null) return undefined;
    if (typeof vd !== 'object') return vd;
    return vd.localValue !== undefined && vd.localValue !== '' ? vd.localValue : vd.syncValue;
  };

  const effectiveCoerced = (vd) => {
    if (vd == null) return undefined;
    if (typeof vd !== 'object') return vd;
    const val = vd.localValue !== undefined && vd.localValue !== '' ? vd.localValue : vd.syncValue;
    if (val === undefined) return undefined;
    return coerceByType(val, vd.type || 'string');
  };

  const makeScope = (scopeKey, seed, readOnly) => {
    const working = {};
    const pending = {};
    // Every key the context seeded, INCLUDING disabled ones. Only 'clear' uses
    // the full list — it nulls every context key, mirroring the Developer engine's
    // walk over the raw context map (see 'clear' below).
    const seededKeys = Object.keys(seed || {});
    // A disabled variable is "skipped during resolution" (variableBaseSchema), so
    // it must read as ABSENT, not as its value: the Developer engine returns
    // undefined from get, false from has, and omits the key from toObject. Leaving
    // it out of 'working' reproduces all three at once, because 'working' is this
    // scope's value store AND its existence store. Safe previously seeded disabled
    // variables unconditionally and leaked their values to scripts (RQ-5691).
    for (const k of seededKeys) {
      const vd = seed[k];
      if (vd && vd.isEnabled === false) continue;
      working[k] = effectiveCoerced(vd);
    }
    const scope = {
      get: (key) => {
        if (pending[key] !== undefined) return coerceByType(pending[key].value, pending[key].type);
        return working[key];
      },
      set: (key, value) => {
        // Standalone requests (no parent collection) get a read-only scope where
        // writes are a silent no-op — not a throw (RQ-4236). A pre-request throw
        // here aborted the send entirely; the write simply having no effect is the
        // intended behavior. In-collection writes still persist.
        if (readOnly) return;
        // null/undefined CLEARS the variable (Postman runner parity, RQ-4780) — get
        // returns undefined, not the string "null". Mirrors unset; guard before
        // rawEntry, which would String()-flatten null to "null".
        if (value == null) {
          delete working[key];
          delete pending[key];
          mutations[scopeKey][key] = null;
          return;
        }
        working[key] = value;
        const entry = rawEntry(value);
        pending[key] = entry;
        mutations[scopeKey][key] = entry;
      },
      unset: (key) => {
        if (readOnly) return;
        delete working[key];
        delete pending[key];
        mutations[scopeKey][key] = null;
      },
      clear: () => {
        if (readOnly) return;
        // Walks the SEEDED keys, not 'working' — disabled variables are no longer
        // seeded into 'working', but clear must still delete them, exactly as the
        // Developer engine's clear walks the whole context map. Keys a script added
        // via set are absent from seededKeys and are caught by the pending pass below.
        for (const k of seededKeys) {
          delete working[k];
          delete pending[k];
          mutations[scopeKey][k] = null;
        }
        for (const k of Object.keys(pending)) {
          if (pending[k] !== null) {
            delete working[k];
            delete pending[k];
            mutations[scopeKey][k] = null;
          }
        }
      },
      has: (key) => Object.prototype.hasOwnProperty.call(working, key),
      toObject: () => Object.assign({}, working),
    };
    return scope;
  };

  // ── rq.response ──
  const buildResponse = (raw) => {
    if (!raw) return null;
    const status = raw.status || 0;
    const statusText = raw.statusText || '';
    const headers = raw.headers || {};
    const body = raw.body || '';
    const time = raw.time || 0;
    const size = raw.size || 0;

    const assertCond = (cond, msg, neg) => {
      const pass = neg ? !cond : cond;
      if (!pass) throw new Error(msg);
    };

    const makeAssertions = (neg) => {
      const be = {
        get ok() { assertCond(status >= 200 && status < 300, 'Expected status 2xx, got ' + status, neg); },
        get success() { assertCond(status >= 200 && status < 300, 'Expected status 2xx, got ' + status, neg); },
        get accepted() { assertCond(status === 202, 'Expected status 202, got ' + status, neg); },
        get info() { assertCond(status >= 100 && status < 200, 'Expected status 1xx, got ' + status, neg); },
        get redirection() { assertCond(status >= 300 && status < 400, 'Expected status 3xx, got ' + status, neg); },
        get clientError() { assertCond(status >= 400 && status < 500, 'Expected status 4xx, got ' + status, neg); },
        get badRequest() { assertCond(status === 400, 'Expected status 400, got ' + status, neg); },
        get unauthorized() { assertCond(status === 401, 'Expected status 401, got ' + status, neg); },
        get forbidden() { assertCond(status === 403, 'Expected status 403, got ' + status, neg); },
        get notFound() { assertCond(status === 404, 'Expected status 404, got ' + status, neg); },
        get rateLimited() { assertCond(status === 429, 'Expected status 429, got ' + status, neg); },
        get serverError() { assertCond(status >= 500 && status < 600, 'Expected status 5xx, got ' + status, neg); },
        get error() { assertCond(status >= 400 && status < 600, 'Expected status 4xx or 5xx, got ' + status, neg); },
      };
      const have = {
        status(expected) {
          if (typeof expected === 'number') assertCond(status === expected, 'Expected status ' + expected + ', got ' + status, neg);
          else assertCond(statusText.toLowerCase() === String(expected).toLowerCase(), 'Expected statusText "' + expected + '", got "' + statusText + '"', neg);
        },
        // Optional 2nd arg asserts the header VALUE (RQ-5663). MUST stay behaviorally
        // identical to the Developer arm in sandbox-definitions/requestResponse.ts —
        // name lookup case-INsensitive, value compare case-SENSITIVE + exact +
        // untrimmed, presence asserted before value, negation on presence ONLY.
        header(name) {
          const found = Object.keys(headers).find((k) => k.toLowerCase() === name.toLowerCase());
          assertCond(found !== undefined, 'Expected header "' + name + '" to be present', neg);
          // found === undefined here means the negated presence arm passed — the
          // header is absent, so there is no value to compare.
          if (arguments.length < 2 || found === undefined) return;
          const expected = arguments[1];
          const actual = headers[found];
          assertCond(actual === expected, 'Expected header "' + name + '" to be "' + expected + '", got "' + actual + '"', neg);
        },
        body(expected) { assertCond(body === expected, 'Expected body to equal "' + expected + '"', neg); },
        jsonBody() {
          // Parity with Developer mode (sandbox-definitions/requestResponse.ts):
          // path lookup and deep-equality go through lodash, so bracket/array-index
          // paths (items[0].id) resolve and object equality is key-order-insensitive.
          // lodash is a SOURCE_BUNDLE package loaded LAZILY here (require-cached), so
          // the no-arg valid-JSON check stays lodash-free and off the hot path.
          if (arguments.length === 0) { try { JSON.parse(body); } catch(e) { assertCond(false, 'Expected response body to be valid JSON', neg); } return; }
          const parsed = JSON.parse(body);
          const _ = globalThis.require('lodash');
          const path = arguments[0];
          const actual = _.get(parsed, path);
          if (arguments.length === 1) { assertCond(actual !== undefined, 'Expected JSON path "' + path + '" to exist', neg); }
          else { assertCond(_.isEqual(actual, arguments[1]), 'Expected JSON path "' + path + '" to equal ' + JSON.stringify(arguments[1]) + ', got ' + JSON.stringify(actual), neg); }
        },
        // jsonSchema (RQ-4233): mirrors the Developer-mode impl in
        // sandbox-definitions/requestResponse.ts. Ajv is a SOURCE_BUNDLE package;
        // load it LAZILY via the isolate require chain on first use, so the ~115KB
        // Ajv eval stays off the hot path for the scripts that never validate a
        // schema. globalThis.require is wired before this shim (boot step 3).
        jsonSchema(schema, options) {
          let parsed;
          // Throw UNCONDITIONALLY on a non-JSON body — matching Developer mode
          // (requestResponse.ts jsonSchema, which throws before negation applies).
          // Using assertCond(false, ..., neg) here would flip under negation, so
          // rq.response.to.not.have.jsonSchema on a non-JSON body would wrongly
          // pass in Safe while Developer throws.
          try { parsed = JSON.parse(body); }
          catch(e) { throw new Error('Expected response body to be valid JSON for schema validation'); }
          const Ajv = globalThis.require('ajv');
          const ajv = new Ajv(options);
          const validate = ajv.compile(schema);
          assertCond(validate(parsed), 'Response body does not match JSON schema', neg);
        },
      };
      const a = { be, have };
      if (!neg) Object.defineProperty(a, 'not', { get() { return makeAssertions(true); } });
      return a;
    };

    // rq.response.headers HYBRID facade (RQ-4233) — the wire headers as
    // ENUMERABLE data properties (so headers['x'] / Object.keys / spread /
    // JSON.stringify keep working exactly as before), PLUS non-enumerable
    // case-insensitive get/has/all. Mirrors buildResponseHeaders in
    // packages/sandbox-definitions/src/requestResponse.ts.
    const headerEntries = Object.keys(headers).map((k) => [k, headers[k]]);
    const headerEq = (a, b) => a.toLowerCase() === b.toLowerCase();
    const responseHeaders = Object.assign({}, headers);
    Object.defineProperties(responseHeaders, {
      get: { value: (name) => { const e = headerEntries.find((pair) => headerEq(pair[0], name)); return e ? e[1] : undefined; }, enumerable: false },
      has: { value: (name) => headerEntries.some((pair) => headerEq(pair[0], name)), enumerable: false },
      all: { value: () => Object.assign({}, headers), enumerable: false },
    });
    Object.freeze(responseHeaders);

    return Object.freeze({
      status,
      code: status,
      statusText,
      headers: responseHeaders,
      body,
      bodyEncoding: raw.bodyEncoding,
      time,
      responseTime: time,
      size,
      json() { return JSON.parse(body); },
      text() { return body; },
      toJSON() { return { status, statusText, headers, body, bodyEncoding: raw.bodyEncoding, time }; },
      get to() { return makeAssertions(false); },
      // rq.response.stream (Postman pm.response.stream parity) — raw body bytes as a
      // Buffer via the SafeBuffer shim (buffer-bridge, installed as globalThis.Buffer).
      // Lazy getter: Buffer is installed by the time a user script reads .stream. Mirrors
      // the Developer engine's stream getter in sandbox-definitions/requestResponse.ts.
      get stream() { return globalThis.Buffer.from(body, raw.bodyEncoding === 'base64' ? 'base64' : 'utf8'); },
    });
  };

  // ── rq.request (mirrors buildScriptHttpRequest — mutable headers facade, ADR-167) ──
  const buildRequest = (raw) => {
    if (!raw) return null;
    // extractBody: GraphQL has .query; HTTP has .body.raw
    const bodyStr = raw.query !== undefined ? raw.query : (raw.body && raw.body.raw ? raw.body.raw : undefined);
    // Working copy of headers as ordered name/value pairs — read accessors and
    // toJSON read this; mutators update it so a script sees its own writes.
    const working = [];
    if (Array.isArray(raw.headers)) {
      for (var h of raw.headers) { if (h && h.key && h.isEnabled !== false) working.push({ name: h.key, value: h.value || '' }); }
    } else if (raw.headers && typeof raw.headers === 'object') {
      for (var hk of Object.keys(raw.headers)) working.push({ name: hk, value: raw.headers[hk] });
    }
    const qp = {};
    if (Array.isArray(raw.queryParams)) {
      for (var q of raw.queryParams) { if (q && q.key && q.isEnabled !== false) qp[q.key] = q.value || ''; }
    }
    const eq = (a, b) => String(a).toLowerCase() === String(b).toLowerCase();
    const headers = {
      add(header) {
        working.push({ name: header.key, value: header.value });
        requestMutations.push({ kind: 'add', name: header.key, value: header.value });
      },
      upsert(header) {
        const existing = working.find((x) => eq(x.name, header.key));
        if (existing) existing.value = header.value;
        else working.push({ name: header.key, value: header.value });
        requestMutations.push({ kind: 'upsert', name: header.key, value: header.value });
      },
      remove(name) {
        for (var i = working.length - 1; i >= 0; i--) { if (eq(working[i].name, name)) working.splice(i, 1); }
        requestMutations.push({ kind: 'remove', name });
      },
      // Postman HeaderList.clear() parity (RQ-3720): removes ALL headers, no-arg
      // by contract — any argument is ignored, matching Postman.
      clear() {
        working.length = 0;
        requestMutations.push({ kind: 'clear' });
      },
      has(name) { return working.some((x) => eq(x.name, name)); },
      get(name) { const f = working.find((x) => eq(x.name, name)); return f ? f.value : undefined; },
      all() { const out = {}; for (var w of working) out[w.name] = w.value; return out; },
    };
    return Object.freeze({
      url: raw.url || '',
      method: raw.method || 'GET',
      headers,
      queryParams: Object.freeze(qp),
      body: bodyStr,
      addHeader(header) { headers.add(header); },
      removeHeader(name) { headers.remove(name); },
      upsertHeader(header) { headers.upsert(header); },
      toJSON() { return { url: this.url, method: this.method, headers: headers.all(), queryParams: qp, body: bodyStr }; },
    });
  };

  // ── rq.info (frozen, eventName attached) ──
  // collectionId is excluded — internal only, not user-facing (ADR-053), matching
  // Developer mode (sandbox-definitions/rqMethods.ts). It is still read off the raw
  // ctx.info below to determine collectionVariables writability.
  const rawInfo = ctx.info || {};
  const info = Object.freeze({
    requestId: rawInfo.requestId,
    requestName: rawInfo.requestName,
    iteration: rawInfo.iteration,
    iterationCount: rawInfo.iterationCount,
    entryIndex: rawInfo.entryIndex,
    totalEntries: rawInfo.totalEntries,
    eventName: rawInfo.eventName ? rawInfo.eventName : undefined,
  });

  // ── collectionVariables read-only determination (ADR-053 parity) ──
  // rq.collectionVariables is writable only when the request belongs to a
  // collection. Standalone requests (collectionId == null) get a read-only
  // scope where set/unset/clear are silent no-ops (RQ-4236) — matching Developer
  // mode (sandbox-definitions/rqMethods.ts), which computes
  // \`context.info.collectionId === null\`. Hardcoding read-only here broke every
  // in-collection script once Safe mode became the default engine.
  const collectionReadOnly = !ctx.info || ctx.info.collectionId == null;

  // ── rq.vault ──
  // When the runtime sets \`ctx.vaultAccessDenial\`, every accessor throws an
  // actionable error rather than silently returning empty. One sentence per
  // reason: 'setting-off' is ADR-196's device setting (RQ-3734, AC-008), and
  // 'no-vault' is an environment with no vault at all, where naming a settings
  // page would point at a surface that does not exist.
  //
  // These two strings MUST stay identical to sandbox-definitions/rqMethods.ts.
  // Safe mode (here) and Developer mode (there) are one product, and a script
  // that reports different text per engine is the drift this package exists to
  // prevent.
  const secrets = ctx.secrets || {};
  const VAULT_ACCESS_DENIAL_MESSAGE = {
    'setting-off':
      'Vault access from scripts is disabled on this device. Turn on "Allow scripts to access vault secrets" in Settings → Vault to read vault secrets in scripts.',
    'no-vault':
      'Vault secrets are not available in this execution environment. They stay on your machine, so run this request from the desktop app to read them in scripts.',
  };
  const assertVaultAccessEnabled = () => {
    const denial = ctx.vaultAccessDenial;
    if (denial) {
      throw new Error(VAULT_ACCESS_DENIAL_MESSAGE[denial]);
    }
  };
  const vault = {
    get(key) {
      assertVaultAccessEnabled();
      const e = secrets[key];
      if (!e || e.isEnabled === false) return undefined;
      return e.localValue !== undefined && e.localValue !== '' ? e.localValue : e.syncValue;
    },
    has(key) {
      assertVaultAccessEnabled();
      const e = secrets[key];
      return !!e && e.isEnabled !== false;
    },
    toObject() {
      assertVaultAccessEnabled();
      const r = {};
      for (const k of Object.keys(secrets)) {
        const v = vault.get(k);
        if (v !== undefined) r[k] = v;
      }
      return r;
    },
  };

  // ── rq.iterationData ──
  const iterData = ctx.iterationData || {};
  const iterationData = {
    get(key) {
      const e = iterData[key];
      if (!e || e.isEnabled === false) return undefined;
      return e.localValue !== undefined && e.localValue !== '' ? e.localValue : e.syncValue;
    },
    has(key) {
      const e = iterData[key];
      return !!e && e.isEnabled !== false;
    },
    toObject() {
      const r = {};
      for (const k of Object.keys(iterData)) {
        const v = iterationData.get(k);
        if (v !== undefined) r[k] = v;
      }
      return r;
    },
  };

  // ── rq.sendRequest ──
  const sendRequest = (input, callback) => {
    const config = typeof input === 'string' ? { url: input } : input;
    if (!config || !config.url) {
      const err = new Error('sendRequest: a non-empty url is required.');
      if (callback) setTimeout(() => callback(err), 0);
      return Promise.reject(err);
    }
    const headers = {};
    if (config.header) {
      if (Array.isArray(config.header)) {
        for (const h of config.header) { if (!h.disabled) headers[h.key] = h.value; }
      } else {
        for (const k of Object.keys(config.header)) headers[k] = config.header[k];
      }
    }
    var body;
    if (config.body) {
      if (config.body.mode === 'raw') body = config.body.raw;
      else if (config.body.mode === 'urlencoded') {
        body = config.body.urlencoded.filter(function(e) { return !e.disabled; }).map(function(e) { return encodeURIComponent(e.key) + '=' + encodeURIComponent(e.value); }).join('&');
        if (!Object.keys(headers).some(function(k) { return k.toLowerCase() === 'content-type'; })) headers['content-type'] = 'application/x-www-form-urlencoded';
      }
    }
    const init = { method: config.method || 'GET', headers: headers };
    if (body !== undefined) init.body = body;
    const start = Date.now();
    const promise = fetch(config.url, init).then(async (raw) => {
      const responseTime = Date.now() - start;
      const rawText = await raw.text();
      const hdr = {};
      if (raw.headers && raw.headers.forEach) raw.headers.forEach((v, k) => { hdr[k.toLowerCase()] = v; });
      const res = {
        code: raw.status,
        status: raw.statusText,
        headers: Object.assign({ get(n) { return hdr[n.toLowerCase()]; } }, hdr),
        responseTime: responseTime,
        json() { return JSON.parse(rawText); },
        text() { return rawText; },
      };
      if (callback) setTimeout(() => callback(null, res), 0);
      return res;
    }, (cause) => {
      const err = new Error('sendRequest: the request could not be sent.');
      err.cause = cause;
      if (callback) setTimeout(() => callback(err), 0);
      throw err;
    });
    return promise;
  };

  // ── rq.cookies ──
  const cookieBridge = globalThis.__rq_cookies;
  const allowlistRaw = JSON.parse(globalThis.__rq_hostAllowlist_json || '[]');
  const allowedHosts = {};
  for (var i = 0; i < allowlistRaw.length; i++) allowedHosts[allowlistRaw[i].toLowerCase()] = true;

  // QuickJS has no URL constructor — extract hostname via regex
  const hostFromUrl = (url) => {
    if (typeof url !== 'string') return null;
    const m = url.match(/^https?:\\/\\/([^/:]+)/i);
    return m && m[1] ? m[1].toLowerCase() : null;
  };
  const fireCb = (cb, err, result) => {
    if (cb) setTimeout(() => cb(err, result), 0);
  };

  const makeCookieJar = () => ({
    set(url, nameOrCookie, valueOrCb, maybeCb) {
      const host = hostFromUrl(url);
      if (!host) { const e = new Error('CookieStore: invalid URL "' + url + '".'); fireCb(typeof valueOrCb === 'function' ? valueOrCb : maybeCb, e); return Promise.reject(e); }
      if (!allowedHosts[host]) { const e = new Error('CookieStore: programmatic access to "' + host + '" is denied.'); fireCb(typeof valueOrCb === 'function' ? valueOrCb : maybeCb, e); return Promise.reject(e); }
      const cb = typeof valueOrCb === 'function' ? valueOrCb : maybeCb;
      var cookie;
      if (typeof nameOrCookie === 'string') {
        cookie = { name: nameOrCookie, value: String(valueOrCb === cb ? '' : valueOrCb), domain: host, path: '/', secure: false, httpOnly: false, expiry: { type: 'session' } };
      } else {
        cookie = { name: nameOrCookie.name, value: nameOrCookie.value, domain: host, path: nameOrCookie.path || '/', secure: !!nameOrCookie.secure, httpOnly: !!nameOrCookie.httpOnly, expiry: nameOrCookie.expiry || { type: 'session' } };
      }
      const res = cookieBridge({ op: 'upsert', host: host, cookie: cookie });
      if (res && res.error) { const e = new Error(res.error); fireCb(cb, e); return Promise.reject(e); }
      fireCb(cb, null, cookie);
      return Promise.resolve(cookie);
    },
    get(url, name, cb) {
      const host = hostFromUrl(url);
      if (!host) { const e = new Error('CookieStore: invalid URL "' + url + '".'); fireCb(cb, e); return Promise.reject(e); }
      if (!allowedHosts[host]) { const e = new Error('CookieStore: programmatic access to "' + host + '" is denied.'); fireCb(cb, e); return Promise.reject(e); }
      const res = cookieBridge({ op: 'list', host: host });
      if (res && res.error) { const e = new Error(res.error); fireCb(cb, e); return Promise.reject(e); }
      const cookies = res.result || [];
      const found = cookies.find(function(c) { return c.name === name; });
      const val = found ? found.value : undefined;
      fireCb(cb, null, val);
      return Promise.resolve(val);
    },
    getAll(url, cb) {
      const host = hostFromUrl(url);
      if (!host) { const e = new Error('CookieStore: invalid URL "' + url + '".'); fireCb(cb, e); return Promise.reject(e); }
      if (!allowedHosts[host]) { const e = new Error('CookieStore: programmatic access to "' + host + '" is denied.'); fireCb(cb, e); return Promise.reject(e); }
      const res = cookieBridge({ op: 'list', host: host });
      if (res && res.error) { const e = new Error(res.error); fireCb(cb, e); return Promise.reject(e); }
      fireCb(cb, null, res.result || []);
      return Promise.resolve(res.result || []);
    },
    unset(url, name, cb) {
      const host = hostFromUrl(url);
      if (!host) { const e = new Error('CookieStore: invalid URL "' + url + '".'); fireCb(cb, e); return Promise.reject(e); }
      if (!allowedHosts[host]) { const e = new Error('CookieStore: programmatic access to "' + host + '" is denied.'); fireCb(cb, e); return Promise.reject(e); }
      cookieBridge({ op: 'remove', host: host, name: name, path: '/' });
      fireCb(cb, null);
      return Promise.resolve();
    },
    clear(url, cb) {
      const host = hostFromUrl(url);
      if (!host) { const e = new Error('CookieStore: invalid URL "' + url + '".'); fireCb(cb, e); return Promise.reject(e); }
      if (!allowedHosts[host]) { const e = new Error('CookieStore: programmatic access to "' + host + '" is denied.'); fireCb(cb, e); return Promise.reject(e); }
      cookieBridge({ op: 'clear', host: host });
      fireCb(cb, null);
      return Promise.resolve();
    },
  });

  // ── rq.execution (flow control — ADR-169, Postman pm.execution parity) ──
  // Hand-written mirror of \`createExecutionNamespace\` (sandbox-definitions): it
  // must produce the BYTE-IDENTICAL ExecutionDirective shape the Developer factory
  // produces (parity enforced by execution-directive.test.ts, not by code sharing).
  // \`location\` arrives in the copied context (ScriptExecutionContext.location);
  // \`phase\` is threaded in as the copied \`__rq_phase\` string ('pre-request' /
  // 'post-response') the host sets before this shim runs.
  const execLocation = Array.isArray(ctx.location) ? ctx.location.slice() : [];
  const execCurrent = execLocation.length > 0 ? execLocation[execLocation.length - 1] : undefined;
  // A real array (so Array.isArray stays true, matching Postman) carrying a
  // read-only \`.current\`, then frozen — same shape as ScriptExecutionLocation.
  execLocation.current = execCurrent;
  Object.freeze(execLocation);
  const isPreRequest = (globalThis.__rq_phase || '') === 'pre-request';
  const execution = {
    setNextRequest(nameOrNull) {
      globalThis.__rq_executionDirective = { kind: 'set-next-request', target: nameOrNull === null ? null : String(nameOrNull) };
    },
    location: execLocation,
  };
  // skipRequest: pre-request only. Sets the directive then throws to abort the
  // rest of the script (Postman parity). The directive is already collected on
  // the global, so the host drain picks it up; the host recognizes a skip-request
  // directive and surfaces a CLEAN result (mirrors the Developer SkipRequestSignal
  // path). In post-response, skipRequest is OMITTED so calling it is
  // 'is not a function' (Postman parity, matching the Developer engine).
  if (isPreRequest) {
    execution.skipRequest = () => {
      globalThis.__rq_executionDirective = { kind: 'skip-request' };
      throw new Error('rq.execution.skipRequest()');
    };
  }

  // ── rq.visualizer (response visualizer — ADR-202, Postman pm.visualizer parity) ──
  // Hand-written mirror of \`createVisualizer\` (sandbox-definitions/visualizer.ts): it
  // must produce the BYTE-IDENTICAL VisualizerOutput shape the Developer factory does
  // (parity enforced by the engine tests, not by code sharing). Handlebars arrives via
  // the in-guest require chain (\`require('handlebars')\` → the internal vendor IIFE),
  // loaded LAZILY inside set() so the ~108KB eval stays off the hot path for scripts
  // that never visualize. Available in BOTH the pre-request and post-response phases
  // (Postman parity, ADR-202 "Amendment (2026-08-02)"): the RQ_COLLECT_EXPR drain reads
  // __rq_visualizerOutput in either, and the runtime lifts the pre-request output onto
  // the entry too, last-writer-wins. Absent in on-message alone (\`isOnMessage\` below),
  // which mirrors the Developer engine's PHASE_RESTRICTED deletion — nothing lifts a
  // per-message visualizer output, so the call would be ignored rather than absent.

  // Hand-written mirror of \`buildScriptMessage\` (sandbox-definitions/requestResponse.ts).
  // On-message only: \`isOnMessage\` gates the namespace absent everywhere else, which is
  // parity with the Developer engine's PHASE_RESTRICTED deletion.
  //
  // THIS FILE IS THE REASON PHASE_RESTRICTED's derivation is not enough. The derived map
  // governs the Developer engine's namespace deletion; the gate below is a separate
  // hand-written boolean the derivation never reaches, in the engine that is the DEFAULT.
  // A member added to \`rqMethods.ts\` and not here exists in Developer mode and not in
  // Safe mode, and neither the compiler nor key-presence parity notices. U-15 diffs the
  // key sets across both files; U-17 pins this member's ABSENCE outside on-message.
  const isOnMessage = (globalThis.__rq_phase || '') === 'on-message';
  // Kept as its OWN boolean rather than reusing \`isOnMessage\` inverted at the literal:
  // two surfaces sharing one gate expression is the silent form of a phase-gating bug,
  // so U-17 asserts the gates are distinct.
  const isVisualizerPhase = !isOnMessage;
  const makeMessage = (raw) => {
    if (!raw) return null;
    // Local, mirroring \`buildResponse\`'s own private \`assertCond\`. Deliberately not
    // hoisted into shared scope: that would edit a working, unrelated code path.
    const assertCond = (cond, msg, neg) => {
      const pass = neg ? !cond : cond;
      if (!pass) throw new Error(msg);
    };
    const parse = () => {
      try {
        return { ok: true, value: JSON.parse(raw.data) };
      } catch (e) {
        return { ok: false, value: undefined };
      }
    };
    const mkAssert = (negate) => {
      const be = {
        get json() {
          assertCond(parse().ok, 'Expected message to be valid JSON', negate);
          return undefined;
        },
        get present() {
          assertCond(raw.data.length > 0, 'Expected message to be non-empty', negate);
          return undefined;
        },
      };
      const have = {
        body: (expected) => {
          assertCond(raw.data.indexOf(expected) !== -1, 'Expected message to include "' + expected + '"', negate);
        },
        jsonBody: (...args) => {
          const parsed = parse();
          if (args.length === 0) {
            assertCond(parsed.ok, 'Expected message to be valid JSON', negate);
            return;
          }
          // With a path, an unparseable payload is an authoring error rather than a
          // failed assertion — matches the Developer factory exactly.
          if (!parsed.ok) throw new Error('Expected message to be valid JSON');
          const path = args[0];
          const actual = require('lodash').get(parsed.value, path);
          if (args.length === 1) {
            assertCond(actual !== undefined, 'Expected message to have path "' + path + '"', negate);
            return;
          }
          assertCond(
            require('lodash').isEqual(actual, args[1]),
            'Expected message path "' + path + '" to equal ' + JSON.stringify(args[1]),
            negate,
          );
        },
      };
      return negate ? { be, have } : { be, have, not: mkAssert(true) };
    };
    return {
      index: raw.index,
      timestamp: raw.timestamp,
      data: raw.data,
      to: mkAssert(false),
      json: () => {
        try {
          return JSON.parse(raw.data);
        } catch (e) {
          throw new Error('Message is not valid JSON');
        }
      },
      text: () => raw.data,
      toJSON: () => ({ index: raw.index, timestamp: raw.timestamp, data: raw.data }),
    };
  };

  const makeVisualizer = () => ({
    set: (template, data) => {
      // 1. JSON-snapshot the data at set() time, guarded — a circular/BigInt throw
      //    records an error rather than aborting the script (FR-10). A bare function
      //    makes JSON.stringify return undefined (no throw) → normalize to 'null'.
      let dataJson;
      try {
        dataJson = JSON.stringify(data === undefined ? null : data);
        if (dataJson === undefined) dataJson = 'null';
      } catch (e) {
        globalThis.__rq_visualizerOutput = { kind: 'error', message: 'rq.visualizer.set() was called with data that could not be serialized to JSON.' };
        return;
      }
      // 2. Compile + render eagerly, guarded — a Handlebars syntax error → error output,
      //    never deferred. The data snapshot is embedded as window.__rq_viz_data__ with
      //    \`<\` escaped so a value containing </script> can't break out of the embed.
      let html;
      try {
        const Handlebars = globalThis.require('handlebars');
        const rendered = Handlebars.compile(template)(JSON.parse(dataJson));
        const embed = '<script>window.__rq_viz_data__ = ' + dataJson.replace(/</g, '\\\\u003c') + ';</script>\\n';
        html = embed + rendered;
      } catch (e) {
        globalThis.__rq_visualizerOutput = { kind: 'error', message: 'rq.visualizer.set() was called with a template that could not be compiled.' };
        return;
      }
      // 3. Overwrite the slot (last-writer-wins is free).
      globalThis.__rq_visualizerOutput = { kind: 'compiled', html: html, data: JSON.parse(dataJson) };
    },
    clear: () => { globalThis.__rq_visualizerOutput = { kind: 'cleared' }; },
  });

  globalThis.rq = {
    test,
    expect: chai ? chai.expect : undefined,
    environment: makeScope('environment', ctx.environment),
    variables: makeScope('runtime', ctx.variables),
    globals: makeScope('global', ctx.global),
    collectionVariables: makeScope('collection', ctx.collectionVariables, collectionReadOnly),
    request: buildRequest(ctx.request),
    response: buildResponse(ctx.response),
    info: info,
    vault,
    iterationData,
    sendRequest,
    cookies: { jar: () => makeCookieJar() },
    isSafeMode: true,
    execution,
    visualizer: isVisualizerPhase ? makeVisualizer() : undefined,
    message: isOnMessage ? makeMessage(globalThis.__rq_message) : undefined,
  };

  // Per-iteration message rebinding for the on-message batch loop (ADR-208 §7,
  // runtime 021 §Decision). The engine drives one iteration per message FROM THE
  // HOST — it does not re-evaluate this shim per message, because the scopes'
  // in-guest \`working\` state is what makes read-your-own-writes hold across a
  // batch. So it needs a way to re-point \`rq.message\` between iterations, and
  // this is it.
  //
  // Gated on \`isOnMessage\` for the same reason the literal above is: a setter
  // that ignored the phase would re-introduce the surface this phase gate exists
  // to keep absent, from a second site the gating test does not read.
  globalThis.__rq_setMessage = (raw) => {
    globalThis.rq.message = isOnMessage ? makeMessage(raw) : undefined;
  };
})();
`;

/**
 * In-isolate JS that clears the per-iteration collectors, run by the engine after
 * it has drained a batch iteration's slice (ADR-208 §7).
 *
 * Clears the ARRAYS in place (`length = 0`) rather than reassigning them: the rq
 * shim's closure holds the same array objects, so a reassignment would leave
 * `rq.test()` pushing into an orphan the drain never reads again.
 *
 * `__rq_mutations` is deliberately NOT cleared. It accumulates across the whole
 * batch and the engine takes its latest snapshot each iteration, which is both
 * what ADR-208 §6's accumulate-and-emit-once asks for and what keeps the
 * mutations a killed iteration cannot have produced out of the result.
 */
export const RQ_ITERATION_RESET_EXPR = `(() => {
  if (globalThis.__rq_testResults) globalThis.__rq_testResults.length = 0;
  if (globalThis.__rq_requestMutations) globalThis.__rq_requestMutations.length = 0;
  globalThis.__rq_error = undefined;
  globalThis.__rq_stack = undefined;
  return true;
})()`;

/**
 * The shape the engine reads back out of the isolate after the script runs.
 * `mutations` matches `RawScopeMutations` from `@requestly/sandbox-definitions`
 * (keys: global / environment / collection / runtime), so the host feeds it
 * straight into `inflateMutations` (ADR-053 Layer 2).
 */
export interface InIsolateCollected {
  readonly testResults: TestResult[];
  readonly mutations: RawScopeMutations;
  /** Request header mutations from `rq.request.headers.*` in call order (ADR-167). */
  readonly requestMutations?: RequestHeaderMutation[];
  /** Flow-control directive from `rq.execution.setNextRequest` / `skipRequest` (ADR-169). */
  readonly executionDirective?: ExecutionDirective;
  /** Visualizer intent from `rq.visualizer.set()` / `clear()` (ADR-202, FR-18). */
  readonly visualizerOutput?: VisualizerDirective;
}

/**
 * In-isolate JS expression that serializes the collected results to a JSON string
 * for the engine to copy out (a single string crosses the edge — copied data).
 */
export const RQ_COLLECT_EXPR = `JSON.stringify({ testResults: globalThis.__rq_testResults || [], mutations: globalThis.__rq_mutations || {}, requestMutations: globalThis.__rq_requestMutations || [], executionDirective: globalThis.__rq_executionDirective, visualizerOutput: globalThis.__rq_visualizerOutput })`;
