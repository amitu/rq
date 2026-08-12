/**
 * Request/Response builders and assertion chain for the rq namespace.
 *
 * Exposes a curated allowlist of request/response properties to user scripts (ADR-054).
 * Protocol-specific interfaces per ADR-136: HTTP/GraphQL and gRPC each get their own
 * ScriptRequest/ScriptResponse shape with native properties and assertion chain.
 * Internal types (KeyValuePair metadata, HttpBody variants) are not leaked.
 */

import { EntryType } from '@requestly/shared-types';
import type {
  GrpcScriptResponse as GrpcScriptResponseData,
  GraphQLResponse,
  HttpResponse,
  ParsedGrpcRequest,
  ParsedGraphQLRequest,
  ParsedHttpRequest,
  ParsedKeyValue,
} from '@requestly/shared-types';
import type { GrpcStreamMessage, RequestHeaderMutation, ScriptMessageInput } from '@requestly/shared-types/runtime';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface AssertionLibs {
  chai: { expect: unknown };
  lodash: { get: (obj: unknown, path: string) => unknown; isEqual: (a: unknown, b: unknown) => boolean };
  ajv: unknown;
  /**
   * Handlebars compiler for `rq.visualizer.set()` (ADR-202). Delivered via the
   * `VENDOR_IIFES` vendor bundle: the Developer engine evals the Handlebars IIFE
   * in the VM realm and hands the resulting compiler in here (structurally the
   * same channel as `chai`/`lodash`/`ajv`). The Safe engine does not use this —
   * its hand-written shim requires Handlebars in-guest.
   */
  handlebars: { compile: (template: string) => (context?: unknown) => string };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Converts parsed key-value pairs to a flat Record.
 * Disabled entries are already filtered at the SDK boundary (ADR-043).
 * Duplicate keys: last value wins (matches HTTP semantics).
 */
function kvpToRecord(kvps: readonly ParsedKeyValue[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const kvp of kvps) {
    record[kvp.key] = kvp.value;
  }
  return record;
}

/**
 * Extracts a raw body string from the request.
 * - HttpRequest: returns body.raw (covers JSON, text, raw content types)
 * - GraphQLRequest: returns the query string
 */
function extractBody(request: ParsedHttpRequest | ParsedGraphQLRequest): string | undefined {
  if ('query' in request) {
    return request.query;
  }
  return request.body.raw || undefined;
}

// ---------------------------------------------------------------------------
// Shared assertion helper
// ---------------------------------------------------------------------------

function assertCondition(condition: boolean, message: string, negate: boolean): void {
  const effective = negate ? !condition : condition;
  if (!effective) {
    throw new Error(negate ? `Not expected: ${message}` : message);
  }
}

// ===================================================================
// HTTP / GraphQL — rq.request and rq.response
// ===================================================================

// ---------------------------------------------------------------------------
// rq.request (HTTP/GraphQL)
// ---------------------------------------------------------------------------

/**
 * Mutable header facade on `rq.request.headers` (ADR-167). Read accessors see the
 * script's own in-flight writes; mutators record ops onto the shared collector
 * AND update the working copy. Header names are matched case-insensitively.
 */
export interface ScriptRequestHeaders {
  add(header: { key: string; value: string }): void;
  upsert(header: { key: string; value: string }): void;
  remove(name: string): void;
  /**
   * Removes ALL headers (Postman `HeaderList.clear()` parity, RQ-3720). Takes no
   * argument; any value passed is ignored — Postman's `clear()` always clears the
   * whole list, never a single header.
   */
  clear(): void;
  has(name: string): boolean;
  get(name: string): string | undefined;
  all(): Record<string, string>;
}

export interface ScriptHttpRequest {
  readonly url: string;
  readonly method: string;
  /** Mutable header facade (ADR-167). Mutations apply pre-request only. */
  readonly headers: ScriptRequestHeaders;
  readonly queryParams: Readonly<Record<string, string>>;
  readonly body: string | undefined;
  /** SDK-style aliases (Postman `pm.request.addHeader/…`) — delegate to `headers.*`. */
  addHeader(header: { key: string; value: string }): void;
  removeHeader(name: string): void;
  upsertHeader(header: { key: string; value: string }): void;
  toJSON(): {
    url: string;
    method: string;
    headers: Record<string, string>;
    queryParams: Record<string, string>;
    body: string | undefined;
  };
}

/** Sink the header facade records mutations onto (ADR-167). */
export interface RequestMutationCollector {
  headers: RequestHeaderMutation[];
}

function buildScriptHttpRequest(
  request: ParsedHttpRequest | ParsedGraphQLRequest,
  collector?: RequestMutationCollector,
): ScriptHttpRequest {
  const url = request.url;
  const method = request.method;
  // Working copy of headers as ordered name/value pairs — read accessors and
  // toJSON read this; mutators update it so a script sees its own writes.
  const working: { name: string; value: string }[] = request.headers.map((kvp) => ({
    name: kvp.key,
    value: kvp.value,
  }));
  const queryParams = kvpToRecord(request.queryParams);
  const body = extractBody(request);

  const eq = (a: string, b: string): boolean => a.toLowerCase() === b.toLowerCase();
  const record = (op: RequestHeaderMutation): void => {
    if (collector) collector.headers.push(op);
  };

  const headers: ScriptRequestHeaders = {
    add(header) {
      working.push({ name: header.key, value: header.value });
      record({ kind: 'add', name: header.key, value: header.value });
    },
    upsert(header) {
      const existing = working.find((h) => eq(h.name, header.key));
      if (existing) existing.value = header.value;
      else working.push({ name: header.key, value: header.value });
      record({ kind: 'upsert', name: header.key, value: header.value });
    },
    remove(name) {
      for (let i = working.length - 1; i >= 0; i--) {
        const entry = working[i];
        if (entry && eq(entry.name, name)) working.splice(i, 1);
      }
      record({ kind: 'remove', name });
    },
    // Postman `HeaderList.clear()` parity (RQ-3720): removes ALL headers and
    // records a clear op. No-arg by contract — any argument the caller passes
    // is ignored, matching Postman where `clear(name)` still clears everything.
    clear() {
      working.length = 0;
      record({ kind: 'clear' });
    },
    has(name) {
      return working.some((h) => eq(h.name, name));
    },
    get(name) {
      return working.find((h) => eq(h.name, name))?.value;
    },
    all() {
      const out: Record<string, string> = {};
      for (const h of working) out[h.name] = h.value;
      return out;
    },
  };

  return Object.freeze({
    url,
    method,
    headers,
    queryParams: Object.freeze(queryParams),
    body,
    addHeader(header: { key: string; value: string }) {
      headers.add(header);
    },
    removeHeader(name: string) {
      headers.remove(name);
    },
    upsertHeader(header: { key: string; value: string }) {
      headers.upsert(header);
    },
    toJSON() {
      return { url, method, headers: headers.all(), queryParams, body };
    },
  });
}

// ---------------------------------------------------------------------------
// HTTP assertion types — explicit interfaces so tsc emits the full shape in
// .d.ts, which the codegen reads to generate editor autocompletion types.
// ---------------------------------------------------------------------------

export interface StatusAssertions {
  readonly ok: void;
  readonly success: void;
  readonly accepted: void;
  readonly info: void;
  readonly redirection: void;
  readonly clientError: void;
  readonly badRequest: void;
  readonly unauthorized: void;
  readonly forbidden: void;
  readonly notFound: void;
  readonly rateLimited: void;
  readonly serverError: void;
  readonly error: void;
}

export interface HaveAssertions {
  status(expected: number | string): void;
  header(name: string): void;
  body(expected: string): void;
  jsonBody(): void;
  jsonBody(path: string): void;
  jsonBody(path: string, value: unknown): void;
  jsonSchema(schema: object, options?: object): void;
}

interface NegatedHttpResponseAssertions {
  readonly be: StatusAssertions;
  readonly have: HaveAssertions;
}

export interface ResponseAssertions {
  readonly be: StatusAssertions;
  readonly have: HaveAssertions;
  readonly not: NegatedHttpResponseAssertions;
}

function createHttpAssertions(
  status: number,
  statusText: string,
  headers: Record<string, string>,
  body: string,
  negate: boolean,
  libs: AssertionLibs,
): ResponseAssertions | NegatedHttpResponseAssertions {
  function statusGetter(condition: boolean, message: string): undefined {
    assertCondition(condition, message, negate);
    return undefined;
  }

  const be = {
    get ok() {
      return statusGetter(status >= 200 && status < 300, `Expected status 2xx, got ${String(status)}`);
    },
    get success() {
      return statusGetter(status >= 200 && status < 300, `Expected status 2xx, got ${String(status)}`);
    },
    get accepted() {
      return statusGetter(status === 202, `Expected status 202, got ${String(status)}`);
    },
    get info() {
      return statusGetter(status >= 100 && status < 200, `Expected status 1xx, got ${String(status)}`);
    },
    get redirection() {
      return statusGetter(status >= 300 && status < 400, `Expected status 3xx, got ${String(status)}`);
    },
    get clientError() {
      return statusGetter(status >= 400 && status < 500, `Expected status 4xx, got ${String(status)}`);
    },
    get badRequest() {
      return statusGetter(status === 400, `Expected status 400, got ${String(status)}`);
    },
    get unauthorized() {
      return statusGetter(status === 401, `Expected status 401, got ${String(status)}`);
    },
    get forbidden() {
      return statusGetter(status === 403, `Expected status 403, got ${String(status)}`);
    },
    get notFound() {
      return statusGetter(status === 404, `Expected status 404, got ${String(status)}`);
    },
    get rateLimited() {
      return statusGetter(status === 429, `Expected status 429, got ${String(status)}`);
    },
    get serverError() {
      return statusGetter(status >= 500 && status < 600, `Expected status 5xx, got ${String(status)}`);
    },
    get error() {
      return statusGetter(status >= 400 && status < 600, `Expected status 4xx or 5xx, got ${String(status)}`);
    },
  };

  const have = {
    status(expected: number | string): void {
      if (typeof expected === 'number') {
        assertCondition(status === expected, `Expected status ${String(expected)}, got ${String(status)}`, negate);
      } else {
        assertCondition(
          statusText.toLowerCase() === expected.toLowerCase(),
          `Expected statusText "${expected}", got "${statusText}"`,
          negate,
        );
      }
    },
    header(name: string): void {
      const found = Object.keys(headers).some((k) => k.toLowerCase() === name.toLowerCase());
      assertCondition(found, `Expected header "${name}" to be present`, negate);
    },
    body(expected: string): void {
      // Postman's `pm.response.to.have.body(str)` asserts full string EQUALITY,
      // not substring containment (verified against a live Postman run). Using
      // `includes` here was a silent pass↔fail bug on migration: a should-fail
      // assertion (body merely contains the string) would go green.
      assertCondition(body === expected, `Expected body to equal "${expected}"`, negate);
    },
    jsonBody(...args: [] | [string] | [string, unknown]): void {
      let parsed: unknown;
      let parseOk = true;
      try {
        parsed = JSON.parse(body);
      } catch (err) {
        if (args.length === 0) {
          parseOk = false;
        } else {
          throw new Error('Expected response body to be valid JSON', { cause: err });
        }
      }

      if (args.length === 0) {
        assertCondition(parseOk, 'Expected response body to be valid JSON', negate);
        return;
      }

      const [path] = args;
      const actual = libs.lodash.get(parsed, path);

      if (args.length === 1) {
        assertCondition(actual !== undefined, `Expected JSON path "${path}" to exist`, negate);
      } else {
        const value = args[1];
        assertCondition(
          libs.lodash.isEqual(actual, value),
          `Expected JSON path "${path}" to equal ${JSON.stringify(value)}, got ${JSON.stringify(actual)}`,
          negate,
        );
      }
    },
    jsonSchema(schema: object, options?: object): void {
      let parsed: unknown;
      try {
        parsed = JSON.parse(body);
      } catch (err) {
        throw new Error('Expected response body to be valid JSON for schema validation', { cause: err });
      }

      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- Ajv constructor is unknown at boundary; shape is known from Ajv library
      const AjvClass = libs.ajv as new (opts?: object) => { compile: (s: object) => (d: unknown) => boolean };
      const ajv = new AjvClass(options);
      const validate = ajv.compile(schema);
      const valid = validate(parsed);
      assertCondition(valid, 'Response body does not match JSON schema', negate);
    },
  };

  const assertion: ResponseAssertions | NegatedHttpResponseAssertions = {
    be,
    have,
  } satisfies NegatedHttpResponseAssertions;

  if (!negate) {
    Object.defineProperty(assertion, 'not', {
      get() {
        return createHttpAssertions(status, statusText, headers, body, true, libs);
      },
      enumerable: true,
      configurable: false,
    });
  }

  return assertion;
}

// ---------------------------------------------------------------------------
// rq.response (HTTP/GraphQL)
// ---------------------------------------------------------------------------

/**
 * Read-only header facade on `rq.response.headers` (RQ-4233). A HYBRID: the wire
 * headers are present as own-enumerable string-keyed data properties (so bracket
 * access `headers['Content-Type']`, `Object.keys`, spread, and `JSON.stringify`
 * all keep working exactly as before this facade existed), PLUS non-enumerable
 * `get`/`has`/`all` methods for case-insensitive lookup (Postman parity for
 * `pm.response.headers.get()`). Mirrors the `rq.sendRequest` response-header
 * shape. The methods are non-enumerable so they never pollute `Object.keys` /
 * `JSON.stringify`. `all()` returns a fresh copy preserving original wire casing.
 */
export interface ScriptResponseHeaders {
  get(name: string): string | undefined;
  has(name: string): boolean;
  all(): Record<string, string>;
  // Wire headers as index-accessible data properties. The value union must cover
  // every member (`get`/`has`/`all` + the string header values) to satisfy the
  // index-signature-covers-all-members rule; mirrors `ScriptHeaderList`
  // (sendRequest.ts). Bracket access to a real header key yields the string value.
  [key: string]:
    | string
    | ((name: string) => string | undefined)
    | ((name: string) => boolean)
    | (() => Record<string, string>)
    | undefined;
}

/**
 * Raw response body bytes exposed on `rq.response.stream` (Postman `pm.response.stream`
 * parity). A Buffer-like handle: `.toString(encoding)` re-encodes the bytes, `.length`
 * is the byte length. Backed by a real Node `Buffer` in the Developer engine and by the
 * Safe-engine `Buffer` shim (buffer-bridge) in the Safe engine — both satisfy this shape.
 * Typed as a minimal self-contained interface (not Node's `Buffer`) so the generated
 * editor `.d.ts` needs no `@types/node`.
 */
export interface ScriptResponseStream {
  toString(encoding?: string): string;
  readonly length: number;
  readonly [index: number]: number;
}

export interface ScriptHttpResponse {
  readonly status: number;
  readonly code: number;
  readonly statusText: string;
  readonly headers: ScriptResponseHeaders;
  readonly body: string;
  /**
   * The raw response body bytes as a Buffer-like handle (Postman `pm.response.stream`
   * parity). Lazily built from `body` + `bodyEncoding`, so scripts that never read it
   * pay nothing. Common use: `rq.response.stream.toString('base64')` to embed a binary
   * response (image/PDF) in a visualization.
   */
  readonly stream: ScriptResponseStream;
  /**
   * How `body` encodes the original bytes (ADR-153): `'base64'` for binary
   * responses (images, PDFs, ...), `'utf8'` for text. Absent on responses
   * persisted before ADR-153 — treat absent as `'utf8'`.
   */
  readonly bodyEncoding?: 'utf8' | 'base64';
  readonly time: number;
  readonly responseTime: number;
  readonly size: number;
  readonly to: ResponseAssertions;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Scripting API: users need dynamic property access on parsed JSON without type narrowing
  json(): any;
  text(): string;
  toJSON(): {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: string;
    bodyEncoding?: 'utf8' | 'base64';
    time: number;
  };
}

/**
 * Builds the hybrid `rq.response.headers` facade (RQ-4233). The wire headers are
 * spread as own-ENUMERABLE data properties, so pre-facade patterns keep working
 * unchanged — `headers['Content-Type']`, `Object.keys(headers)`, `{ ...headers }`,
 * and `JSON.stringify(headers)` all see exactly the header record. On top, the
 * case-insensitive `get`/`has`/`all` methods are attached as NON-enumerable, so
 * they never appear in `Object.keys` / `JSON.stringify`. Mirrors the
 * `rq.sendRequest` response-header shape.
 */
function buildResponseHeaders(headers: Record<string, string>): ScriptResponseHeaders {
  const eq = (a: string, b: string): boolean => a.toLowerCase() === b.toLowerCase();
  const entries = Object.entries(headers);
  // Method layer, declared against the interface so no cast is needed (mirrors
  // sendRequest.ts toHeaderList). Defined non-enumerable below so the methods
  // never show up in Object.keys / JSON.stringify.
  const facade: ScriptResponseHeaders = {
    get: (name: string): string | undefined => entries.find(([key]) => eq(key, name))?.[1],
    has: (name: string): boolean => entries.some(([key]) => eq(key, name)),
    all: (): Record<string, string> => ({ ...headers }),
  };
  // Enumerable data layer: the raw wire headers as own string-keyed properties
  // (preserves original casing). The index signature permits string assignment.
  for (const [key, value] of entries) facade[key] = value;
  // Make the three methods non-enumerable so JSON.stringify / Object.keys see
  // only the header record.
  for (const method of ['get', 'has', 'all']) {
    Object.defineProperty(facade, method, { enumerable: false });
  }
  return Object.freeze(facade);
}

function buildScriptHttpResponse(response: HttpResponse | GraphQLResponse, libs: AssertionLibs): ScriptHttpResponse {
  const { status, statusText, headers, body, time } = response;
  const size = response.size;
  // GraphQLResponse has no bodyEncoding (always JSON text); HttpResponse may
  // lack it on pre-ADR-153 persisted data. Absent means 'utf8' either way.
  const bodyEncoding = 'bodyEncoding' in response ? response.bodyEncoding : undefined;

  const responseObj: Record<string, unknown> = {
    status,
    code: status,
    statusText,
    headers: buildResponseHeaders(headers),
    body,
    bodyEncoding,
    time,
    responseTime: time,
    size,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Scripting API: dynamic JSON access
    json(): any {
      return JSON.parse(body);
    },
    text() {
      return body;
    },
    toJSON() {
      return { status, statusText, headers, body, bodyEncoding, time };
    },
  };

  Object.defineProperty(responseObj, 'to', {
    get() {
      return createHttpAssertions(status, statusText, headers, body, false, libs);
    },
    enumerable: true,
    configurable: false,
  });

  // rq.response.stream (Postman pm.response.stream parity) — the raw body bytes as a
  // Buffer, built lazily from `body` + `bodyEncoding`. The Developer engine runs
  // host-side in node:vm, so globalThis.Buffer is the real Node Buffer; the Safe engine
  // builds its own equivalent via the SafeBuffer shim (isolated-rq.ts). Non-enumerable
  // so it never pollutes Object.keys / JSON.stringify (toJSON already omits it).
  Object.defineProperty(responseObj, 'stream', {
    get() {
      return globalThis.Buffer.from(body, bodyEncoding === 'base64' ? 'base64' : 'utf8');
    },
    enumerable: false,
    configurable: false,
  });

  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- runtime shape matches ScriptHttpResponse; Record<string, unknown> used for Object.defineProperty compatibility
  return Object.freeze(responseObj) as unknown as ScriptHttpResponse;
}

// ===================================================================
// gRPC — rq.request and rq.response (ADR-136)
// ===================================================================

// ---------------------------------------------------------------------------
// rq.request (gRPC)
// ---------------------------------------------------------------------------

export interface ScriptGrpcRequest {
  readonly url: string;
  readonly methodPath: string;
  readonly metadata: Readonly<Record<string, string>>;
  readonly message: string;
  readonly auth: Readonly<Record<string, unknown>> | null;
  toJSON(): {
    url: string;
    methodPath: string;
    metadata: Record<string, string>;
    message: string;
  };
}

function buildScriptGrpcRequest(request: ParsedGrpcRequest, auth: Record<string, unknown> | null): ScriptGrpcRequest {
  const url = request.url;
  const methodPath = request.methodPath;
  const metadata = kvpToRecord(request.metadata);
  const message = request.message;

  return Object.freeze({
    url,
    methodPath,
    metadata: Object.freeze(metadata),
    message,
    auth: auth ? Object.freeze({ ...auth }) : null,
    toJSON() {
      return { url, methodPath, metadata, message };
    },
  });
}

// ---------------------------------------------------------------------------
// gRPC assertion types (ADR-136 §3)
// ---------------------------------------------------------------------------

export interface GrpcStatusAssertions {
  readonly ok: void;
  readonly success: void;
  readonly error: void;
}

export interface GrpcHaveAssertions {
  status(expected: number): void;
  metadata(name: string): void;
  trailer(name: string): void;
  message(expected: string): void;
  jsonMessage(): void;
  jsonMessage(path: string): void;
  jsonMessage(path: string, value: unknown): void;
  jsonSchema(schema: object, options?: object): void;
}

interface NegatedGrpcResponseAssertions {
  readonly be: GrpcStatusAssertions;
  readonly have: GrpcHaveAssertions;
}

export interface GrpcResponseAssertions {
  readonly be: GrpcStatusAssertions;
  readonly have: GrpcHaveAssertions;
  readonly not: NegatedGrpcResponseAssertions;
}

function createGrpcAssertions(
  statusCode: number,
  metadataRecord: Record<string, string>,
  trailersRecord: Record<string, string>,
  lastMessageBody: string,
  negate: boolean,
  libs: AssertionLibs,
): GrpcResponseAssertions | NegatedGrpcResponseAssertions {
  function statusGetter(condition: boolean, msg: string): undefined {
    assertCondition(condition, msg, negate);
    return undefined;
  }

  const be = {
    get ok() {
      return statusGetter(statusCode === 0, `Expected gRPC status 0 (OK), got ${String(statusCode)}`);
    },
    get success() {
      return statusGetter(statusCode === 0, `Expected gRPC status 0 (OK), got ${String(statusCode)}`);
    },
    get error() {
      return statusGetter(statusCode !== 0, `Expected gRPC error status (non-zero), got ${String(statusCode)}`);
    },
  };

  const have = {
    status(expected: number): void {
      assertCondition(
        statusCode === expected,
        `Expected gRPC status ${String(expected)}, got ${String(statusCode)}`,
        negate,
      );
    },
    metadata(name: string): void {
      const found = Object.keys(metadataRecord).some((k) => k.toLowerCase() === name.toLowerCase());
      assertCondition(found, `Expected metadata "${name}" to be present`, negate);
    },
    trailer(name: string): void {
      const found = Object.keys(trailersRecord).some((k) => k.toLowerCase() === name.toLowerCase());
      assertCondition(found, `Expected trailer "${name}" to be present`, negate);
    },
    message(expected: string): void {
      assertCondition(lastMessageBody.includes(expected), `Expected last message to include "${expected}"`, negate);
    },
    jsonMessage(...args: [] | [string] | [string, unknown]): void {
      let parsed: unknown;
      let parseOk = true;
      try {
        parsed = JSON.parse(lastMessageBody);
      } catch (err) {
        if (args.length === 0) {
          parseOk = false;
        } else {
          throw new Error('Expected last message to be valid JSON', { cause: err });
        }
      }

      if (args.length === 0) {
        assertCondition(parseOk, 'Expected last message to be valid JSON', negate);
        return;
      }

      const [path] = args;
      const actual = libs.lodash.get(parsed, path);

      if (args.length === 1) {
        assertCondition(actual !== undefined, `Expected JSON path "${path}" to exist in last message`, negate);
      } else {
        const value = args[1];
        assertCondition(
          libs.lodash.isEqual(actual, value),
          `Expected JSON path "${path}" to equal ${JSON.stringify(value)}, got ${JSON.stringify(actual)}`,
          negate,
        );
      }
    },
    jsonSchema(schema: object, options?: object): void {
      let parsed: unknown;
      try {
        parsed = JSON.parse(lastMessageBody);
      } catch (err) {
        throw new Error('Expected last message to be valid JSON for schema validation', { cause: err });
      }

      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- Ajv constructor is unknown at boundary; shape is known from Ajv library
      const AjvClass = libs.ajv as new (opts?: object) => { compile: (s: object) => (d: unknown) => boolean };
      const ajv = new AjvClass(options);
      const validate = ajv.compile(schema);
      const valid = validate(parsed);
      assertCondition(valid, 'Last message does not match JSON schema', negate);
    },
  };

  const assertion: GrpcResponseAssertions | NegatedGrpcResponseAssertions = {
    be,
    have,
  } satisfies NegatedGrpcResponseAssertions;

  if (!negate) {
    Object.defineProperty(assertion, 'not', {
      get() {
        return createGrpcAssertions(statusCode, metadataRecord, trailersRecord, lastMessageBody, true, libs);
      },
      enumerable: true,
      configurable: false,
    });
  }

  return assertion;
}

// ---------------------------------------------------------------------------
// rq.response (gRPC) — ADR-136 §2
// ---------------------------------------------------------------------------

export interface ScriptGrpcResponse {
  readonly statusCode: number;
  readonly statusMessage: string;
  /** Read facade (RQ-4233) — get/has/all, case-insensitive. Matches HTTP `rq.response.headers`. */
  readonly metadata: ScriptResponseHeaders;
  /** Read facade (RQ-4233) — get/has/all, case-insensitive. Matches HTTP `rq.response.headers`. */
  readonly trailers: ScriptResponseHeaders;
  readonly messages: readonly GrpcStreamMessage[];
  readonly responseTime: number;
  readonly to: GrpcResponseAssertions;
  json(): unknown;
  text(): string;
  toJSON(): {
    statusCode: number;
    statusMessage: string;
    metadata: Record<string, string>;
    trailers: Record<string, string>;
    messages: GrpcStreamMessage[];
    responseTime: number;
  };
}

function buildScriptGrpcResponse(response: GrpcScriptResponseData, libs: AssertionLibs): ScriptGrpcResponse {
  const { statusCode, statusMessage, metadata, trailers, messages, responseTime } = response;
  const lastMsg = messages[messages.length - 1];
  const lastMessage = lastMsg ? lastMsg.data : '';

  const responseObj: Record<string, unknown> = {
    statusCode,
    statusMessage,
    metadata: buildResponseHeaders(metadata),
    trailers: buildResponseHeaders(trailers),
    messages: Object.freeze(messages.map((m) => Object.freeze({ ...m }))),
    responseTime,
    json(): unknown {
      if (messages.length === 0) {
        throw new Error('No messages received — cannot parse JSON');
      }
      return JSON.parse(lastMessage);
    },
    text(): string {
      if (messages.length === 0) {
        throw new Error('No messages received');
      }
      return lastMessage;
    },
    toJSON() {
      return { statusCode, statusMessage, metadata, trailers, messages, responseTime };
    },
  };

  Object.defineProperty(responseObj, 'to', {
    get() {
      return createGrpcAssertions(statusCode, metadata, trailers, lastMessage, false, libs);
    },
    enumerable: true,
    configurable: false,
  });

  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- runtime shape matches ScriptGrpcResponse; Record<string, unknown> used for Object.defineProperty compatibility
  return Object.freeze(responseObj) as unknown as ScriptGrpcResponse;
}

// ===================================================================
// Union types (ADR-136 §4)
// ===================================================================

/** Union of all protocol-specific request types. Prefer `ScriptHttpRequest` or `ScriptGrpcRequest` when the protocol is known. */
export type ScriptRequest = ScriptHttpRequest | ScriptGrpcRequest;
/** Union of all protocol-specific response types. Prefer `ScriptHttpResponse` or `ScriptGrpcResponse` when the protocol is known. */
export type ScriptResponse = ScriptHttpResponse | ScriptGrpcResponse;

// ===================================================================
// Protocol-dispatching builders (ADR-136 §5)
// ===================================================================

/**
 * Auth context extracted from the entry for gRPC script request building.
 * gRPC auth lives on the entry (`GrpcApiEntry.auth`), not the request.
 */
export interface GrpcBuildContext {
  auth: Record<string, unknown> | null;
}

export function buildScriptRequest(
  request: ParsedHttpRequest | ParsedGraphQLRequest | ParsedGrpcRequest,
  entryType: EntryType,
  grpcContext?: GrpcBuildContext,
  collector?: RequestMutationCollector,
): ScriptRequest {
  switch (entryType) {
    case EntryType.grpc:
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees ParsedGrpcRequest
      return buildScriptGrpcRequest(request as ParsedGrpcRequest, grpcContext?.auth ?? null);
    default:
      // Header mutation (ADR-167) is HTTP/GraphQL only; gRPC has no header facade.
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees HTTP/GraphQL
      return buildScriptHttpRequest(request as ParsedHttpRequest | ParsedGraphQLRequest, collector);
  }
}

export function buildScriptResponse(
  response: HttpResponse | GraphQLResponse | GrpcScriptResponseData,
  libs: AssertionLibs,
  entryType: EntryType,
): ScriptResponse {
  switch (entryType) {
    case EntryType.grpc:
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees GrpcScriptResponseData
      return buildScriptGrpcResponse(response as GrpcScriptResponseData, libs);
    default:
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees HTTP/GraphQL
      return buildScriptHttpResponse(response as HttpResponse | GraphQLResponse, libs);
  }
}

// ─── rq.message — the on-message script surface (ADR-208) ────────────────────

/** Assertions on the message currently being handled. */
export interface MessageHaveAssertions {
  /** The message text includes `expected`. */
  body(expected: string): void;
  /**
   * The message parses as JSON. With a dotted `path`, that path exists; with a
   * `value` too, it equals `value`.
   */
  jsonBody(...args: [] | [string] | [string, unknown]): void;
}

export interface MessageBeAssertions {
  /** The message parses as JSON. */
  readonly json: undefined;
  /** The message is non-empty. */
  readonly present: undefined;
}

interface NegatedMessageAssertions {
  readonly be: MessageBeAssertions;
  readonly have: MessageHaveAssertions;
}

export interface MessageAssertions {
  readonly be: MessageBeAssertions;
  readonly have: MessageHaveAssertions;
  readonly not: NegatedMessageAssertions;
}

/**
 * One streamed message, as scripts see it (`rq.message`).
 *
 * `index` is the message's position in the CONNECTION, not in the drain batch —
 * batching is an implementation detail the user never observes, and the index is
 * what ties an assertion or an error back to the message they can see.
 */
export interface ScriptMessage {
  /** Zero-based position in the connection. Users see `msg index + 1`. */
  readonly index: number;
  /** Arrival time, epoch millis. */
  readonly timestamp: number;
  /** Raw payload as delivered by the transport. */
  readonly data: string;
  readonly to: MessageAssertions;
  /** Parsed payload. Throws if the message is not valid JSON. */
  json(): unknown;
  /** Raw payload — the symmetric peer of `json()`, matching `rq.response.text()`. */
  text(): string;
  toJSON(): { index: number; timestamp: number; data: string };
}

function createMessageAssertions(
  data: string,
  negate: boolean,
  libs: AssertionLibs,
): MessageAssertions | NegatedMessageAssertions {
  function parse(): { ok: boolean; value: unknown } {
    try {
      return { ok: true, value: JSON.parse(data) };
    } catch {
      return { ok: false, value: undefined };
    }
  }

  const be = {
    get json(): undefined {
      assertCondition(parse().ok, 'Expected message to be valid JSON', negate);
      return undefined;
    },
    get present(): undefined {
      assertCondition(data.length > 0, 'Expected message to be non-empty', negate);
      return undefined;
    },
  };

  const have: MessageHaveAssertions = {
    body(expected: string): void {
      assertCondition(data.includes(expected), `Expected message to include "${expected}"`, negate);
    },
    jsonBody(...args: [] | [string] | [string, unknown]): void {
      const parsed = parse();

      if (args.length === 0) {
        assertCondition(parsed.ok, 'Expected message to be valid JSON', negate);
        return;
      }

      // With a path, an unparseable message is an authoring error rather than a
      // failed assertion — `not.have.jsonBody('a.b')` should mean "that path is
      // absent", not "the payload happened to be unparseable".
      if (!parsed.ok) {
        throw new Error('Expected message to be valid JSON');
      }

      const [path, expected] = args;
      const actual = libs.lodash.get(parsed.value, path);

      if (args.length === 1) {
        assertCondition(actual !== undefined, `Expected message to have path "${path}"`, negate);
        return;
      }
      assertCondition(
        libs.lodash.isEqual(actual, expected),
        `Expected message path "${path}" to equal ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
        negate,
      );
    },
  };

  return negate ? { be, have } : { be, have, not: createMessageAssertions(data, true, libs) };
}

/** Build the `rq.message` surface for one iteration of an on-message script. */
export function buildScriptMessage(message: ScriptMessageInput, libs: AssertionLibs): ScriptMessage {
  const assertions = createMessageAssertions(message.data, false, libs);

  return {
    index: message.index,
    timestamp: message.timestamp,
    data: message.data,
    // `createMessageAssertions(_, false, _)` always returns the un-negated shape,
    // which carries `not`. Narrowed by construction rather than asserted.
    to: 'not' in assertions ? assertions : { ...assertions, not: assertions },
    json(): unknown {
      try {
        return JSON.parse(message.data);
      } catch (err) {
        throw new Error('Message is not valid JSON', { cause: err });
      }
    },
    text(): string {
      return message.data;
    },
    toJSON() {
      return { index: message.index, timestamp: message.timestamp, data: message.data };
    },
  };
}
