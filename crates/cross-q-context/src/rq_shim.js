// The rq.* namespace shim (docs/CONTEXT.md §2), installed into a bare QuickJS realm.
//
// SCAFFOLD SUBSET: the variable scopes, tests, a minimal `expect`, request/response reads,
// console capture, and the execution directive — enough to prove the runtime + wire contract
// end-to-end. The full surface (real Chai, cookies, sendRequest, the response `.to` assertion
// tree, gRPC, runRequest) is ported from @requestly/sandbox-definitions on top of this.
//
// Contract with the host: reads `__RQ_CONTEXT_JSON` (a JSON string) + `__RQ_PHASE`; accumulates
// outputs on reserved globals the host drains with one JSON.stringify.
(function () {
  'use strict';

  var ctx = JSON.parse(globalThis.__RQ_CONTEXT_JSON || '{}');
  var phase = globalThis.__RQ_PHASE || 'pre-request';

  // --- reserved output channels (drained host-side) ---
  var tests = [];
  var logs = [];
  var mut = { environment: {}, globals: {}, collection: {}, runtime: {} };
  var directive = { value: null };
  globalThis.__rq_tests = tests;
  globalThis.__rq_logs = logs;
  globalThis.__rq_mut = mut;
  globalThis.__rq_directive = directive;

  // --- console capture ---
  function logAt(level) {
    return function () {
      logs.push({ level: level, args: Array.prototype.slice.call(arguments) });
    };
  }
  globalThis.console = { log: logAt('log'), info: logAt('info'), warn: logAt('warn'), error: logAt('error'), debug: logAt('debug') };

  // --- variable scopes (§2.3) ---
  // `displayName` drives the empty-key error; `bucket` is the mutation channel key.
  function makeScope(displayName, bucket, initial, readOnly) {
    var store = Object.assign(Object.create(null), initial || {});
    var scope = {
      get: function (k) { return Object.prototype.hasOwnProperty.call(store, k) ? store[k] : undefined; },
      has: function (k) { return Object.prototype.hasOwnProperty.call(store, k); },
      toObject: function () {
        var o = {};
        for (var k in store) { if (Object.prototype.hasOwnProperty.call(store, k)) o[k] = String(store[k]); }
        return o;
      },
    };
    if (!readOnly) {
      scope.set = function (k, v) {
        if (typeof k !== 'string' || k === '') throw new Error(displayName + ' variable key must be a non-empty string');
        store[k] = v;
        mut[bucket][k] = v;
      };
      scope.unset = function (k) { delete store[k]; mut[bucket][k] = null; };
      scope.clear = function () {
        for (var k in store) { if (Object.prototype.hasOwnProperty.call(store, k)) mut[bucket][k] = null; }
        store = Object.create(null);
      };
    }
    return scope;
  }

  // --- minimal expect (STUB — replaced by injected Chai) ---
  function expect(actual) {
    function fail(msg) { throw new Error(msg); }
    var chain = {};
    var negate = false;
    function check(cond, msg) { if (negate ? cond : !cond) fail(msg); }
    chain.equal = function (e) { check(actual === e, 'expected ' + JSON.stringify(actual) + ' to equal ' + JSON.stringify(e)); return chain; };
    chain.eql = function (e) { check(JSON.stringify(actual) === JSON.stringify(e), 'expected deep equality'); return chain; };
    chain.a = chain.an = function (t) { check(typeof actual === t, 'expected type ' + t); return chain; };
    chain.include = function (e) { check(actual != null && String(actual).indexOf(e) !== -1, 'expected to include ' + e); return chain; };
    chain.property = function (p) { check(actual != null && p in actual, 'expected property ' + p); return chain; };
    Object.defineProperty(chain, 'to', { get: function () { return chain; } });
    Object.defineProperty(chain, 'be', { get: function () { return chain; } });
    Object.defineProperty(chain, 'have', { get: function () { return chain; } });
    Object.defineProperty(chain, 'not', { get: function () { negate = !negate; return chain; } });
    Object.defineProperty(chain, 'ok', { get: function () { check(!!actual, 'expected value to be ok'); return chain; } });
    Object.defineProperty(chain, 'true', { get: function () { check(actual === true, 'expected true'); return chain; } });
    Object.defineProperty(chain, 'false', { get: function () { check(actual === false, 'expected false'); return chain; } });
    Object.defineProperty(chain, 'null', { get: function () { check(actual === null, 'expected null'); return chain; } });
    Object.defineProperty(chain, 'undefined', { get: function () { check(actual === undefined, 'expected undefined'); return chain; } });
    return chain;
  }

  // --- rq.request / rq.response (reads; mutation facade is a later port) ---
  var request = ctx.request || {};
  var response = phase === 'pre-request' ? null : buildResponse(ctx.response || {});
  function buildResponse(r) {
    return {
      status: r.status, code: r.status, statusText: r.statusText, headers: r.headers || {},
      body: r.body, time: r.time, responseTime: r.time,
      text: function () { return typeof r.body === 'string' ? r.body : JSON.stringify(r.body); },
      json: function () { return typeof r.body === 'string' ? JSON.parse(r.body) : r.body; },
    };
  }

  // --- the rq namespace ---
  var rq = {
    isSafeMode: true,
    info: Object.freeze(ctx.info || {}),
    expect: expect,
    environment: makeScope('environment', 'environment', ctx.environment, false),
    globals: makeScope('globals', 'globals', ctx.globals, false),
    collectionVariables: makeScope('collectionVariables', 'collection', ctx.collectionVariables, false),
    variables: makeScope('variables', 'runtime', ctx.variables, false),
    request: request,
    response: response,
    execution: {
      setNextRequest: function (n) { directive.value = { kind: 'set-next-request', target: n === null ? null : String(n) }; },
      skipRequest: function () {
        if (phase !== 'pre-request') throw new Error('skipRequest() is only available in pre-request scripts');
        directive.value = { kind: 'skip-request' };
      },
    },
  };
  rq.test = function (name, fn) {
    try { fn(); tests.push({ name: name, status: 'passed' }); }
    catch (e) { tests.push({ name: name, status: 'failed', error: String((e && e.message) || e) }); }
  };
  rq.test.skip = function (name) { tests.push({ name: name, status: 'skipped' }); };

  globalThis.rq = rq;
})();
