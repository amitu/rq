/**
 * In-isolate shim for Bruno's script API — `bru.*`, `req`, `res`, and the bare `test`/`expect`
 * a `tests {}` block is written against.
 *
 * WHY A SHIM AND NOT A TRANSFORM. Postman is reconciled by rewriting the source
 * (`cq-transform`), and it has to be: v1.0 scripts say `tests['ok'] = responseCode.code === 200`,
 * an *assignment to a subscript*, which no runtime object can intercept. Bruno has no such
 * syntax-level forms — its API is entirely objects and calls — so a shim maps it one-to-one,
 * and does so for aliased use (`const b = bru; b.setVar(…)`) that a static rewrite would miss.
 *
 * THE SURFACE IS THE ONE PEOPLE ACTUALLY USE. It was taken by counting `bru.*`/`req.*`/`res.*`
 * across usebruno's own 223-request test collection, most-used first: `res.getBody`, `res.body`,
 * `bru.setVar`, `req.headerList`, `res.status`, `bru.getVar`, `bru.getEnvVar`, `bru.cookies`,
 * and so on down.
 *
 * ANYTHING NOT MAPPED THROWS, by name. A shim that silently returned `undefined` for
 * `bru.getFolderVar` would turn a missing feature into a wrong result three lines later, which
 * is the failure mode this whole project is organised against.
 */
export const BRU_ISOLATE_SHIM = `
(() => {
  const rq = globalThis.rq;
  if (!rq) return;

  const unsupported = (name) => () => {
    throw new Error(
      name + ' is a Bruno API rq does not implement yet — the script stopped here rather ' +
      'than continuing with a wrong value'
    );
  };

  // --- variables ---------------------------------------------------------------------
  // Bruno's "runtime" vars are rq's; its env/global/collection scopes line up by name.
  const scopeOf = (s) => ({
    get: (k) => s.get(k),
    set: (k, v) => s.set(k, v),
    all: () => (typeof s.toObject === 'function' ? s.toObject() : {}),
  });
  const runtime = scopeOf(rq.variables);
  const env = scopeOf(rq.environment);
  const globals = scopeOf(rq.globals);
  const collection = scopeOf(rq.collectionVariables);

  const bru = {
    // runtime
    getVar: (k) => runtime.get(k),
    setVar: (k, v) => runtime.set(k, v),
    deleteVar: (k) => rq.variables.unset(k),
    getAllVars: () => runtime.all(),

    // environment
    getEnvVar: (k) => env.get(k),
    setEnvVar: (k, v) => env.set(k, v),
    deleteEnvVar: (k) => rq.environment.unset(k),
    getAllEnvVars: () => env.all(),
    getEnvName: () => (rq.info && rq.info.environmentName) || undefined,

    // global environment
    getGlobalEnvVar: (k) => globals.get(k),
    setGlobalEnvVar: (k, v) => globals.set(k, v),
    getAllGlobalEnvVars: () => globals.all(),

    // collection
    getCollectionVar: (k) => collection.get(k),
    setCollectionVar: (k, v) => collection.set(k, v),
    getAllCollectionVars: () => collection.all(),

    // Folder and request scopes are read-only in Bruno and both resolve out of the same
    // pool rq exposes as variables; a miss falls through rather than inventing a value.
    getFolderVar: (k) => runtime.get(k),
    getRequestVar: (k) => runtime.get(k),

    // flow control
    setNextRequest: (name) => rq.execution.setNextRequest(name),
    runner: {
      skipRequest: () => rq.execution.skipRequest(),
      stopExecution: () => rq.execution.setNextRequest(null),
    },

    // requests
    sendRequest: (config, cb) => rq.sendRequest(config, cb),
    runRequest: (path) => rq.sendRequest(path),

    cookies: rq.cookies,
    isSafeMode: rq.isSafeMode === true,
    cwd: unsupported('bru.cwd()'),
    // Resolve {{name}} against the variable scopes, rq's precedence (runtime > environment >
    // collection > global). An unresolved template is left literal — matching how rq sends an
    // unprovided {{TOKEN}} rather than fabricating a value. Flat names only: dot-paths, recursive
    // resolution, and $dynamic vars ($guid, …) need the host resolver the guest realm can't reach.
    interpolate: (str) => {
      if (typeof str !== 'string') return str;
      return str.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (match, name) => {
        for (const scope of [runtime, env, collection, globals]) {
          const v = scope.get(name);
          if (v !== undefined && v !== null) return String(v);
        }
        return match;
      });
    },
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  };

  // --- res ---------------------------------------------------------------------------
  const parsed = () => {
    try {
      return rq.response.json();
    } catch (e) {
      return rq.response.text();
    }
  };
  const headerObject = (h) => {
    const out = {};
    if (h && typeof h.toObject === 'function') return h.toObject();
    if (h && typeof h.forEach === 'function') { h.forEach((v, k) => { out[k] = v; }); return out; }
    return h || out;
  };
  let resBody = undefined;
  const res = {
    get status() { return rq.response.code; },
    getStatus: () => rq.response.code,
    get statusText() { return rq.response.statusText; },
    get body() { return resBody !== undefined ? resBody : parsed(); },
    getBody: () => (resBody !== undefined ? resBody : parsed()),
    setBody: (b) => { resBody = b; },
    get responseTime() { return rq.response.responseTime; },
    getResponseTime: () => rq.response.responseTime,
    getHeader: (n) => rq.response.headers.get(n),
    getHeaders: () => headerObject(rq.response.headers),
    get headers() { return headerObject(rq.response.headers); },
    // Bruno hands \`headerList\` back as [{ name, value }].
    get headerList() {
      const o = headerObject(rq.response.headers);
      return Object.keys(o).map((name) => ({ name, value: o[name] }));
    },
  };

  // --- req ---------------------------------------------------------------------------
  const req = {
    getUrl: () => rq.request.url,
    setUrl: (u) => { rq.request.url = u; },
    getMethod: () => rq.request.method,
    setMethod: (m) => { rq.request.method = m; },
    getHeader: (n) => rq.request.headers.get(n),
    getHeaders: () => headerObject(rq.request.headers),
    setHeader: (n, v) => rq.request.headers.upsert({ key: n, value: v }),
    setHeaders: (o) => { for (const k of Object.keys(o || {})) rq.request.headers.upsert({ key: k, value: o[k] }); },
    get headers() { return headerObject(rq.request.headers); },
    get headerList() {
      const o = headerObject(rq.request.headers);
      return Object.keys(o).map((name) => ({ name, value: o[name] }));
    },
    getBody: () => rq.request.body,
    setBody: (b) => { rq.request.body = b; },
    get body() { return rq.request.body; },
    getTimeout: unsupported('req.getTimeout()'),
    setTimeout: unsupported('req.setTimeout()'),
    setMaxRedirects: unsupported('req.setMaxRedirects()'),
  };

  globalThis.bru = bru;
  globalThis.req = req;
  globalThis.res = res;
  // A Bruno \`tests {}\` block calls these bare.
  if (typeof globalThis.test !== 'function') globalThis.test = (name, fn) => rq.test(name, fn);
  if (typeof globalThis.expect !== 'function') globalThis.expect = rq.expect;
})();
`;
