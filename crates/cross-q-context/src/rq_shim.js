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
  var reqmut = []; // ordered request-header ops (§6 requestMutationDiff)
  globalThis.__rq_tests = tests;
  globalThis.__rq_logs = logs;
  globalThis.__rq_mut = mut;
  globalThis.__rq_directive = directive;
  globalThis.__rq_reqmut = reqmut;

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

  // --- rq.request (§2.4) — a header facade whose mutations are recorded to be applied before
  // the request fires (drained as requestMutationDiff). ---
  function buildRequest(rw) {
    var r = rw || {};
    var headers = Array.isArray(r.headers) ? r.headers.map(function (h) { return { key: h.key, value: h.value }; }) : [];
    function idx(name) {
      var l = String(name).toLowerCase();
      for (var i = 0; i < headers.length; i++) { if (String(headers[i].key).toLowerCase() === l) return i; }
      return -1;
    }
    var facade = {
      add: function (h) { headers.push({ key: h.key, value: h.value }); reqmut.push({ op: 'add', key: h.key, value: h.value }); },
      upsert: function (h) { var i = idx(h.key); if (i >= 0) headers[i] = { key: h.key, value: h.value }; else headers.push({ key: h.key, value: h.value }); reqmut.push({ op: 'upsert', key: h.key, value: h.value }); },
      remove: function (name) { var i = idx(name); if (i >= 0) headers.splice(i, 1); reqmut.push({ op: 'remove', name: name }); },
      clear: function () { headers = []; reqmut.push({ op: 'clear' }); },
      has: function (name) { return idx(name) >= 0; },
      get: function (name) { var i = idx(name); return i >= 0 ? headers[i].value : undefined; },
      all: function () { return headers.slice(); },
    };
    return {
      url: r.url, method: r.method, body: r.body, queryParams: r.queryParams, headers: facade,
      addHeader: function (h) { facade.add(h); },
      upsertHeader: function (h) { facade.upsert(h); },
      removeHeader: function (n) { facade.remove(n); },
      toJSON: function () { return { url: r.url, method: r.method, headers: headers.slice(), body: r.body }; },
    };
  }

  // --- rq.response (§2.5) — reads + the `.to.be.*` / `.to.have.*` assertion tree (throws on
  // mismatch; negatable via `.to.not`). ---
  function buildResponse(r) {
    function headerGet(name) {
      var hs = r.headers || {}, l = String(name).toLowerCase();
      for (var k in hs) { if (k.toLowerCase() === l) return hs[k]; }
      return undefined;
    }
    function bodyText() { return typeof r.body === 'string' ? r.body : JSON.stringify(r.body); }
    function bodyJson() { return typeof r.body === 'string' ? JSON.parse(r.body) : r.body; }
    function assertion(neg) {
      function check(cond, msg) { if (neg ? cond : !cond) throw new Error(msg); }
      var status = r.status;
      var be = {};
      function cls(name, test) { Object.defineProperty(be, name, { get: function () { check(test(status), 'expected status ' + status + (neg ? ' not' : '') + ' to be ' + name); return be; } }); }
      cls('ok', function (s) { return s >= 200 && s < 300; });
      cls('success', function (s) { return s >= 200 && s < 300; });
      cls('accepted', function (s) { return s === 202; });
      cls('info', function (s) { return s >= 100 && s < 200; });
      cls('redirection', function (s) { return s >= 300 && s < 400; });
      cls('clientError', function (s) { return s >= 400 && s < 500; });
      cls('badRequest', function (s) { return s === 400; });
      cls('unauthorized', function (s) { return s === 401; });
      cls('forbidden', function (s) { return s === 403; });
      cls('notFound', function (s) { return s === 404; });
      cls('rateLimited', function (s) { return s === 429; });
      cls('serverError', function (s) { return s >= 500 && s < 600; });
      cls('error', function (s) { return s >= 400 && s < 600; });
      var have = {
        status: function (v) { if (typeof v === 'string') check(r.statusText === v, 'expected statusText ' + v); else check(status === v, 'expected status ' + v + ' got ' + status); return have; },
        header: function (name) { check(headerGet(name) !== undefined, 'expected header ' + name); return have; },
        body: function (expected) { check(bodyText() === expected, 'expected body equality'); return have; },
        jsonBody: function (path, value) {
          var j; try { j = bodyJson(); } catch (e) { check(false, 'expected a JSON body'); return have; }
          if (arguments.length === 0) { check(j != null, 'expected a JSON body'); return have; }
          var cur = j, parts = String(path).split('.');
          for (var i = 0; i < parts.length; i++) { cur = cur == null ? undefined : cur[parts[i]]; }
          if (arguments.length >= 2) check(JSON.stringify(cur) === JSON.stringify(value), 'expected jsonBody ' + path + ' to equal');
          else check(cur !== undefined, 'expected jsonBody ' + path);
          return have;
        },
      };
      var root = {};
      Object.defineProperty(root, 'be', { get: function () { return be; } });
      Object.defineProperty(root, 'have', { get: function () { return have; } });
      Object.defineProperty(root, 'not', { get: function () { return assertion(!neg); } });
      return root;
    }
    return {
      status: r.status, code: r.status, statusText: r.statusText, headers: r.headers || {},
      body: r.body, time: r.time, responseTime: r.time, size: r.size,
      text: function () { return bodyText(); },
      json: function () { return bodyJson(); },
      get to() { return assertion(false); },
    };
  }

  var request = buildRequest(ctx.request);
  var response = phase === 'pre-request' ? null : buildResponse(ctx.response || {});

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
