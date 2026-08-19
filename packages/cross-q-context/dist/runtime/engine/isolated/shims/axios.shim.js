/**
 * In-realm `axios` facade for Bruno scripts — `require('axios')` backed by `rq.sendRequest`.
 *
 * WHY A FACADE, NOT THE PACKAGE. Bruno ships axios as an inbuilt library and its scripts do
 * `const axios = require('axios')`. The real package can't run here: the sandbox exposes no
 * `http`/`net`, so axios has no transport. `rq.sendRequest` IS the sanctioned HTTP path (host
 * fetch, cookie jar, SSRF guard), so this maps the slice of the axios surface Bruno scripts use
 * onto it — request/response shape, `params`, JSON/urlencoded bodies, `axios.create`, and axios's
 * defining behaviour: it REJECTS on a non-2xx status (with `err.response`), unlike `sendRequest`.
 *
 * Realm-agnostic like the `bru` shim: it only reads `globalThis.rq` and installs `globalThis.__axios`,
 * so both engines evaluate the identical source and cannot drift. `require('axios')` returns it
 * (isolated: a `bridge` require entry; developer: a lazy realm-global lookup).
 */
export const AXIOS_ISOLATE_SHIM = `
(() => {
  const rq = globalThis.rq;
  if (!rq || typeof rq.sendRequest !== 'function') return;

  // URLSearchParams isn't present in every realm (the QuickJS isolate has none), so the facade
  // never relies on it — query strings are encoded by hand and this guard gates any USP branch.
  const hasUSP = typeof globalThis.URLSearchParams === 'function';
  const isUSP = (v) => hasUSP && v instanceof globalThis.URLSearchParams;

  // Merge a request config over instance defaults (headers merged key-wise, params shallow-merged).
  const mergeConfig = (defaults, cfg) => {
    const d = defaults || {}, c = cfg || {};
    return {
      ...d, ...c,
      baseURL: c.baseURL !== undefined ? c.baseURL : d.baseURL,
      headers: { ...(d.headers || {}), ...(c.headers || {}) },
      params: { ...(d.params || {}), ...(c.params || {}) },
    };
  };

  const buildUrl = (config) => {
    let url = String(config.url || '');
    if (config.baseURL && !/^https?:\\/\\//i.test(url)) {
      url = String(config.baseURL).replace(/\\/+$/, '') + '/' + url.replace(/^\\/+/, '');
    }
    const params = config.params;
    if (params && typeof params === 'object') {
      let qs;
      if (isUSP(params)) {
        qs = params.toString();
      } else {
        const parts = [];
        const enc = (s) => encodeURIComponent(String(s));
        for (const k of Object.keys(params)) {
          const val = params[k];
          if (val === undefined || val === null) continue;
          if (Array.isArray(val)) { for (const v of val) parts.push(enc(k) + '=' + enc(v)); }
          else parts.push(enc(k) + '=' + enc(val));
        }
        qs = parts.join('&');
      }
      if (qs) url += (url.indexOf('?') === -1 ? '?' : '&') + qs;
    }
    return url;
  };

  // axios data → rq.sendRequest body, defaulting the Content-Type the way axios does.
  const buildBody = (config) => {
    const data = config.data;
    if (data === undefined || data === null) return { body: undefined, contentType: undefined };
    if (isUSP(data)) {
      const urlencoded = [];
      data.forEach((value, key) => urlencoded.push({ key, value: String(value) }));
      return { body: { mode: 'urlencoded', urlencoded }, contentType: 'application/x-www-form-urlencoded' };
    }
    if (typeof data === 'string') {
      return { body: { mode: 'raw', raw: data }, contentType: undefined };
    }
    // Plain object/array → JSON, matching axios's default transformRequest.
    return { body: { mode: 'raw', raw: JSON.stringify(data) }, contentType: 'application/json' };
  };

  const headerGet = (headers, name) => {
    if (!headers) return undefined;
    if (typeof headers.get === 'function') return headers.get(name);
    const lower = name.toLowerCase();
    for (const k of Object.keys(headers)) if (k.toLowerCase() === lower) return headers[k];
    return undefined;
  };

  const toHeaderObject = (headers) => {
    const out = {};
    if (!headers) return out;
    if (typeof headers.toObject === 'function') return headers.toObject();
    for (const k of Object.keys(headers)) { if (typeof headers[k] !== 'function') out[k] = headers[k]; }
    return out;
  };

  const parseData = (res) => {
    const ct = String(headerGet(res.headers, 'content-type') || '');
    if (ct.indexOf('application/json') !== -1) { try { return res.json(); } catch (e) { return res.text; } }
    return res.text;
  };

  const request = async (config) => {
    const headers = { ...(config.headers || {}) };
    const { body, contentType } = buildBody(config);
    if (contentType && headerGet(headers, 'content-type') === undefined) headers['Content-Type'] = contentType;

    const sendConfig = {
      url: buildUrl(config),
      method: String(config.method || 'get').toUpperCase(),
      header: headers,
    };
    if (body !== undefined) sendConfig.body = body;

    const res = await rq.sendRequest(sendConfig);
    const response = {
      data: parseData(res),
      status: res.code,
      statusText: res.status,
      headers: toHeaderObject(res.headers),
      config,
    };

    const validate = typeof config.validateStatus === 'function'
      ? config.validateStatus
      : (s) => s >= 200 && s < 300;
    if (!validate(response.status)) {
      const err = new Error('Request failed with status code ' + response.status);
      err.isAxiosError = true;
      err.config = config;
      err.response = response;
      err.status = response.status;
      throw err;
    }
    return response;
  };

  const makeInstance = (defaults) => {
    const instance = (configOrUrl, config) =>
      request(mergeConfig(defaults, typeof configOrUrl === 'string' ? { ...config, url: configOrUrl } : configOrUrl));
    instance.request = (config) => request(mergeConfig(defaults, config));
    for (const m of ['get', 'delete', 'head', 'options']) {
      instance[m] = (url, config) => request(mergeConfig(defaults, { ...config, url, method: m }));
    }
    for (const m of ['post', 'put', 'patch']) {
      instance[m] = (url, data, config) => request(mergeConfig(defaults, { ...config, url, data, method: m }));
    }
    instance.create = (instanceDefaults) => makeInstance(mergeConfig(defaults, instanceDefaults));
    // Interceptors are not supported (no transport pipeline to hook); expose no-op registries so a
    // script that registers one doesn't crash — it simply has no effect.
    const noopInterceptor = { use: () => 0, eject: () => {} };
    instance.interceptors = { request: noopInterceptor, response: noopInterceptor };
    instance.defaults = defaults;
    return instance;
  };

  globalThis.__axios = makeInstance({});

  // Expose it through require('axios'). Both engines have installed globalThis.require by now
  // (the require chain runs before this shim); wrap it so 'axios' returns the facade and every
  // other id delegates unchanged. Keeping this here (rather than a requireTable entry) keeps the
  // whole facade — logic and wiring — in one realm-agnostic source both engines evaluate.
  const baseRequire = globalThis.require;
  if (typeof baseRequire === 'function') {
    globalThis.require = (id) => (id === 'axios' ? globalThis.__axios : baseRequire(id));
  }
})();
`;
