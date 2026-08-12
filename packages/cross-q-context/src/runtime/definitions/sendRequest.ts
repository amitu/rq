/**
 * `rq.sendRequest()` — sandbox scripting surface (ADR-153, Postman `pm.sendRequest` parity).
 *
 * Wraps the already-injected `fetch` VM global (see GLOBAL_NAMES) so a user
 * script can issue an HTTP sub-request and use the response in the same
 * execution. `fetch` is passed in (default `globalThis.fetch`) so the module
 * stays platform-agnostic and unit-testable with a mock.
 *
 * Dual form (D-11): every call returns a Promise AND invokes an optional
 * Node-style callback, so `await rq.sendRequest(...)` and
 * `rq.sendRequest(..., (err, res) => {})` both work — mirroring `cookies.ts`.
 *
 * Error contract (TB EC-15/EC-16): a transport/network failure rejects /
 * fires the callback with a `kind`-tagged `SendRequestError`; an HTTP 4xx/5xx
 * is NOT an error — `err` is null and the response is delivered. The raw
 * `fetch` `TypeError` is wrapped, never allowed to escape bare.
 */

// ---------------------------------------------------------------------------
// Request input
// ---------------------------------------------------------------------------

/** Headers accepted on the request: object form OR Postman's array form. */
export type SendRequestHeaders =
  | Record<string, string>
  | ReadonlyArray<{ key: string; value: string; disabled?: boolean }>;

/** Request body — discriminated on `mode` (raw | urlencoded in v1). */
export type SendRequestBody =
  | { mode: 'raw'; raw: string }
  | { mode: 'urlencoded'; urlencoded: ReadonlyArray<{ key: string; value: string; disabled?: boolean }> };

/** Config form of the request. */
export interface SendRequestConfig {
  url: string;
  method?: string;
  header?: SendRequestHeaders;
  body?: SendRequestBody;
}

/** A bare URL string or a full config. */
export type SendRequestInput = string | SendRequestConfig;

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/**
 * Response headers exposed to scripts. Mirrors Postman's `HeaderList`: a
 * case-insensitive `get(name)` plus index access, so both
 * `res.headers.get('content-type')` (pasted Postman scripts) and
 * `res.headers['content-type']` (new scripts) work.
 */
export interface ScriptHeaderList {
  get(name: string): string | undefined;
  [key: string]: string | ((name: string) => string | undefined) | undefined;
}

/** Postman-shaped response object (ADR-153 §API Signature). */
export interface SendRequestResponse {
  /** Numeric HTTP status, e.g. 200. */
  code: number;
  /** HTTP status text, e.g. "OK". */
  status: string;
  headers: ScriptHeaderList;
  /** Round-trip time in milliseconds. */
  responseTime: number;
  /** Parsed JSON body. Throws (SyntaxError) on a non-JSON body, like Postman. */
  json(): unknown;
  /** Raw response body as text. */
  text(): string;
}

/** Node-style callback: `(err, response)`. */
export type SendRequestCallback = (err: SendRequestErrors | null, response?: SendRequestResponse) => void;

/** The `rq.sendRequest` callable (dual form). */
export interface ScriptSendRequest {
  (input: SendRequestInput): Promise<SendRequestResponse>;
  (input: SendRequestInput, callback: SendRequestCallback): Promise<SendRequestResponse>;
}

// ---------------------------------------------------------------------------
// Errors (kind-tagged — gr-discriminated-errors-at-boundaries)
// ---------------------------------------------------------------------------

/** Surfaced when the request config has no usable URL. */
export class SendRequestInvalidArgs extends Error {
  readonly kind = 'send-request-invalid-args' as const;

  constructor() {
    super('sendRequest: a non-empty url is required.');
    this.name = 'SendRequestInvalidArgs';
  }
}

/** Surfaced when the underlying fetch fails at the transport level (EC-15). */
export class SendRequestError extends Error {
  readonly kind = 'send-request-network-error' as const;
  readonly url: string;

  constructor(url: string, cause: unknown) {
    super('sendRequest: the request could not be sent.');
    this.name = 'SendRequestError';
    this.url = url;
    // Assigned rather than passed as the `cause` constructor option: the spec makes
    // the option NON-enumerable, so `JSON.stringify(err)` — which is how script logs
    // are serialized on their way out of the sandbox — silently dropped it. A script
    // author doing the natural `console.error(err)` saw `{kind, name, url}` and no
    // reason at all, while the Safe engine (which assigns) showed the reason. This
    // makes the two engines agree and the underlying failure visible (RQ-5318).
    this.cause = cause;
  }
}

/** Discriminated union of every error `rq.sendRequest` can surface. */
export type SendRequestErrors = SendRequestInvalidArgs | SendRequestError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fireCallback(
  callback: SendRequestCallback | undefined,
  err: SendRequestErrors | null,
  result?: SendRequestResponse,
): void {
  if (!callback) return;
  // Next-microtask so a synchronous throw inside the user's callback can't
  // tear through our call-site. Matches the cookies.ts convention.
  queueMicrotask(() => callback(err, result));
}

function normalizeInput(input: SendRequestInput): SendRequestConfig {
  return typeof input === 'string' ? { url: input } : input;
}

/** Folds either header form into a plain record, skipping `disabled: true`. */
function normalizeHeaders(header: SendRequestHeaders | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  if (!header) return out;
  if (Array.isArray(header)) {
    for (const entry of header) {
      if (entry.disabled === true) continue;
      out[entry.key] = entry.value;
    }
    return out;
  }
  // Object form. (Array.isArray narrows the union to the record branch here.)
  for (const [k, v] of Object.entries(header)) {
    out[k] = v;
  }
  return out;
}

/** Builds the fetch body + any auto content-type for the chosen body mode. */
function buildBody(body: SendRequestBody | undefined): { body?: string; contentType?: string } {
  if (!body) return {};
  if (body.mode === 'raw') {
    return { body: body.raw };
  }
  // urlencoded
  const params = new URLSearchParams();
  for (const entry of body.urlencoded) {
    if (entry.disabled === true) continue;
    params.append(entry.key, entry.value);
  }
  return { body: params.toString(), contentType: 'application/x-www-form-urlencoded' };
}

/** Has a header key case-insensitively. */
function hasHeaderCi(headers: Record<string, string>, name: string): boolean {
  const lower = name.toLowerCase();
  return Object.keys(headers).some((k) => k.toLowerCase() === lower);
}

/** Builds the Postman-shaped HeaderList from the fetch Response headers. */
function toHeaderList(responseHeaders: Headers): ScriptHeaderList {
  const lookup: Record<string, string> = {};
  responseHeaders.forEach((value, key) => {
    lookup[key.toLowerCase()] = value;
  });
  const list: ScriptHeaderList = {
    get(name: string): string | undefined {
      return lookup[name.toLowerCase()];
    },
  };
  // Index access mirror — also lower-cased keys so [...] and get() agree.
  for (const [k, v] of Object.entries(lookup)) {
    list[k] = v;
  }
  return list;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/**
 * Builds the `rq.sendRequest` callable. `fetchImpl` defaults to the injected
 * `fetch` global; tests pass a mock.
 */
export function createSendRequest(fetchImpl: typeof globalThis.fetch = globalThis.fetch): ScriptSendRequest {
  function sendRequest(input: SendRequestInput, callback?: SendRequestCallback): Promise<SendRequestResponse> {
    const config = normalizeInput(input);

    if (!config.url) {
      const err = new SendRequestInvalidArgs();
      fireCallback(callback, err);
      return Promise.reject(err);
    }

    const headers = normalizeHeaders(config.header);
    const { body, contentType } = buildBody(config.body);
    if (contentType && !hasHeaderCi(headers, 'content-type')) {
      headers['content-type'] = contentType;
    }

    const init: RequestInit = {
      method: config.method ?? 'GET',
      headers,
    };
    if (body !== undefined) init.body = body;

    const start = performance.now();

    const promise = fetchImpl(config.url, init).then(
      async (raw): Promise<SendRequestResponse> => {
        const responseTime = performance.now() - start;
        const rawText = await raw.text();
        const headerList = toHeaderList(raw.headers);
        const response: SendRequestResponse = {
          code: raw.status,
          status: raw.statusText,
          headers: headerList,
          responseTime,
          // Lazy parse — throws SyntaxError on non-JSON, like Postman's .json().
          json(): unknown {
            return JSON.parse(rawText);
          },
          text(): string {
            return rawText;
          },
        };
        fireCallback(callback, null, response);
        return response;
      },
      (cause: unknown): never => {
        // Transport failure (EC-15): wrap the raw fetch TypeError in a
        // kind-tagged error so scripts can discriminate it. gr-no-silent-catch:
        // we re-throw a typed error, never swallow.
        const err = new SendRequestError(config.url, cause);
        fireCallback(callback, err);
        throw err;
      },
    );

    return promise;
  }

  return sendRequest;
}
