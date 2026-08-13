/**
 * Request/Response builders and assertion chain for the rq namespace.
 *
 * Exposes a curated allowlist of request/response properties to user scripts (ADR-054).
 * Protocol-specific interfaces per ADR-136: HTTP/GraphQL and gRPC each get their own
 * ScriptRequest/ScriptResponse shape with native properties and assertion chain.
 * Internal types (KeyValuePair metadata, HttpBody variants) are not leaked.
 */
import { EntryType } from './_deps.js';
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
/**
 * Converts parsed key-value pairs to a flat Record.
 * Disabled entries are already filtered at the SDK boundary (ADR-043).
 * Duplicate keys: last value wins (matches HTTP semantics).
 */
function kvpToRecord(kvps) {
    const record = {};
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
function extractBody(request) {
    if ('query' in request) {
        return request.query;
    }
    return request.body.raw || undefined;
}
// ---------------------------------------------------------------------------
// Shared assertion helper
// ---------------------------------------------------------------------------
function assertCondition(condition, message, negate) {
    const effective = negate ? !condition : condition;
    if (!effective) {
        throw new Error(negate ? `Not expected: ${message}` : message);
    }
}
function buildScriptHttpRequest(request, collector) {
    const url = request.url;
    const method = request.method;
    // Working copy of headers as ordered name/value pairs — read accessors and
    // toJSON read this; mutators update it so a script sees its own writes.
    const working = request.headers.map((kvp) => ({
        name: kvp.key,
        value: kvp.value,
    }));
    const queryParams = kvpToRecord(request.queryParams);
    const body = extractBody(request);
    const eq = (a, b) => a.toLowerCase() === b.toLowerCase();
    const record = (op) => {
        if (collector)
            collector.headers.push(op);
    };
    const headers = {
        add(header) {
            working.push({ name: header.key, value: header.value });
            record({ kind: 'add', name: header.key, value: header.value });
        },
        upsert(header) {
            const existing = working.find((h) => eq(h.name, header.key));
            if (existing)
                existing.value = header.value;
            else
                working.push({ name: header.key, value: header.value });
            record({ kind: 'upsert', name: header.key, value: header.value });
        },
        remove(name) {
            for (let i = working.length - 1; i >= 0; i--) {
                const entry = working[i];
                if (entry && eq(entry.name, name))
                    working.splice(i, 1);
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
            const out = {};
            for (const h of working)
                out[h.name] = h.value;
            return out;
        },
    };
    return Object.freeze({
        url,
        method,
        headers,
        queryParams: Object.freeze(queryParams),
        body,
        addHeader(header) {
            headers.add(header);
        },
        removeHeader(name) {
            headers.remove(name);
        },
        upsertHeader(header) {
            headers.upsert(header);
        },
        toJSON() {
            return { url, method, headers: headers.all(), queryParams, body };
        },
    });
}
function createHttpAssertions(status, statusText, headers, body, negate, libs) {
    function statusGetter(condition, message) {
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
        status(expected) {
            if (typeof expected === 'number') {
                assertCondition(status === expected, `Expected status ${String(expected)}, got ${String(status)}`, negate);
            }
            else {
                assertCondition(statusText.toLowerCase() === expected.toLowerCase(), `Expected statusText "${expected}", got "${statusText}"`, negate);
            }
        },
        // Postman's `pm.response.to.have.header(name[, value])` takes an OPTIONAL second argument that
        // asserts the header's VALUE (RQ-5663). Dropping it made the assertion strictly more lenient
        // than Postman's, so a should-fail migrated test went green — the same silent pass↔fail failure
        // mode as `to.have.body`. Semantics below are from a live Postman run (PostmanRuntime 7.54.0):
        //   - header NAME lookup is case-INsensitive
        //   - header VALUE compare is case-SENSITIVE, exact, and NOT trimmed
        //   - presence is asserted BEFORE the value
        //   - negation applies to PRESENCE ONLY — the value argument is ignored on the `.not` arm.
        header(name, ...rest) {
            const found = Object.keys(headers).find((k) => k.toLowerCase() === name.toLowerCase());
            assertCondition(found !== undefined, `Expected header "${name}" to be present`, negate);
            // `found === undefined` here means the negated presence arm passed — the header is absent, so
            // there is no value to compare.
            if (rest.length === 0 || found === undefined)
                return;
            const expected = rest[0];
            const actual = headers[found];
            assertCondition(actual === expected, `Expected header "${name}" to be "${expected}", got "${actual}"`, negate);
        },
        body(expected) {
            // Postman's `pm.response.to.have.body(str)` asserts full string EQUALITY,
            // not substring containment (verified against a live Postman run). Using
            // `includes` here was a silent pass↔fail bug on migration: a should-fail
            // assertion (body merely contains the string) would go green.
            assertCondition(body === expected, `Expected body to equal "${expected}"`, negate);
        },
        jsonBody(...args) {
            let parsed;
            let parseOk = true;
            try {
                parsed = JSON.parse(body);
            }
            catch (err) {
                if (args.length === 0) {
                    parseOk = false;
                }
                else {
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
            }
            else {
                const value = args[1];
                assertCondition(libs.lodash.isEqual(actual, value), `Expected JSON path "${path}" to equal ${JSON.stringify(value)}, got ${JSON.stringify(actual)}`, negate);
            }
        },
        jsonSchema(schema, options) {
            let parsed;
            try {
                parsed = JSON.parse(body);
            }
            catch (err) {
                throw new Error('Expected response body to be valid JSON for schema validation', { cause: err });
            }
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- Ajv constructor is unknown at boundary; shape is known from Ajv library
            const AjvClass = libs.ajv;
            const ajv = new AjvClass(options);
            const validate = ajv.compile(schema);
            const valid = validate(parsed);
            assertCondition(valid, 'Response body does not match JSON schema', negate);
        },
    };
    const assertion = {
        be,
        have,
    };
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
/**
 * Builds the hybrid `rq.response.headers` facade (RQ-4233). The wire headers are
 * spread as own-ENUMERABLE data properties, so pre-facade patterns keep working
 * unchanged — `headers['Content-Type']`, `Object.keys(headers)`, `{ ...headers }`,
 * and `JSON.stringify(headers)` all see exactly the header record. On top, the
 * case-insensitive `get`/`has`/`all` methods are attached as NON-enumerable, so
 * they never appear in `Object.keys` / `JSON.stringify`. Mirrors the
 * `rq.sendRequest` response-header shape.
 */
function buildResponseHeaders(headers) {
    const eq = (a, b) => a.toLowerCase() === b.toLowerCase();
    const entries = Object.entries(headers);
    // Method layer, declared against the interface so no cast is needed (mirrors
    // sendRequest.ts toHeaderList). Defined non-enumerable below so the methods
    // never show up in Object.keys / JSON.stringify.
    const facade = {
        get: (name) => entries.find(([key]) => eq(key, name))?.[1],
        has: (name) => entries.some(([key]) => eq(key, name)),
        all: () => ({ ...headers }),
    };
    // Enumerable data layer: the raw wire headers as own string-keyed properties
    // (preserves original casing). The index signature permits string assignment.
    for (const [key, value] of entries)
        facade[key] = value;
    // Make the three methods non-enumerable so JSON.stringify / Object.keys see
    // only the header record.
    for (const method of ['get', 'has', 'all']) {
        Object.defineProperty(facade, method, { enumerable: false });
    }
    return Object.freeze(facade);
}
function buildScriptHttpResponse(response, libs) {
    const { status, statusText, headers, body, time } = response;
    const size = response.size;
    // GraphQLResponse has no bodyEncoding (always JSON text); HttpResponse may
    // lack it on pre-ADR-153 persisted data. Absent means 'utf8' either way.
    const bodyEncoding = 'bodyEncoding' in response ? response.bodyEncoding : undefined;
    const responseObj = {
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
        json() {
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
            // `Buffer` is provided by the host (real Node Buffer in Developer/node:vm; the SafeBuffer
            // shim in the Safe engine). Typed here without pulling @types/node into this browser-capable
            // package.
            const hostBuffer = globalThis.Buffer;
            return hostBuffer.from(body, bodyEncoding === 'base64' ? 'base64' : 'utf8');
        },
        enumerable: false,
        configurable: false,
    });
    // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- runtime shape matches ScriptHttpResponse; Record<string, unknown> used for Object.defineProperty compatibility
    return Object.freeze(responseObj);
}
function buildScriptGrpcRequest(request, auth) {
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
function createGrpcAssertions(statusCode, metadataRecord, trailersRecord, lastMessageBody, negate, libs) {
    function statusGetter(condition, msg) {
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
        status(expected) {
            assertCondition(statusCode === expected, `Expected gRPC status ${String(expected)}, got ${String(statusCode)}`, negate);
        },
        metadata(name) {
            const found = Object.keys(metadataRecord).some((k) => k.toLowerCase() === name.toLowerCase());
            assertCondition(found, `Expected metadata "${name}" to be present`, negate);
        },
        trailer(name) {
            const found = Object.keys(trailersRecord).some((k) => k.toLowerCase() === name.toLowerCase());
            assertCondition(found, `Expected trailer "${name}" to be present`, negate);
        },
        message(expected) {
            assertCondition(lastMessageBody.includes(expected), `Expected last message to include "${expected}"`, negate);
        },
        jsonMessage(...args) {
            let parsed;
            let parseOk = true;
            try {
                parsed = JSON.parse(lastMessageBody);
            }
            catch (err) {
                if (args.length === 0) {
                    parseOk = false;
                }
                else {
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
            }
            else {
                const value = args[1];
                assertCondition(libs.lodash.isEqual(actual, value), `Expected JSON path "${path}" to equal ${JSON.stringify(value)}, got ${JSON.stringify(actual)}`, negate);
            }
        },
        jsonSchema(schema, options) {
            let parsed;
            try {
                parsed = JSON.parse(lastMessageBody);
            }
            catch (err) {
                throw new Error('Expected last message to be valid JSON for schema validation', { cause: err });
            }
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- Ajv constructor is unknown at boundary; shape is known from Ajv library
            const AjvClass = libs.ajv;
            const ajv = new AjvClass(options);
            const validate = ajv.compile(schema);
            const valid = validate(parsed);
            assertCondition(valid, 'Last message does not match JSON schema', negate);
        },
    };
    const assertion = {
        be,
        have,
    };
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
function buildScriptGrpcResponse(response, libs) {
    const { statusCode, statusMessage, metadata, trailers, messages, responseTime } = response;
    const lastMsg = messages[messages.length - 1];
    const lastMessage = lastMsg ? lastMsg.data : '';
    const responseObj = {
        statusCode,
        statusMessage,
        metadata: buildResponseHeaders(metadata),
        trailers: buildResponseHeaders(trailers),
        messages: Object.freeze(messages.map((m) => Object.freeze({ ...m }))),
        responseTime,
        json() {
            if (messages.length === 0) {
                throw new Error('No messages received — cannot parse JSON');
            }
            return JSON.parse(lastMessage);
        },
        text() {
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
    return Object.freeze(responseObj);
}
export function buildScriptRequest(request, entryType, grpcContext, collector) {
    switch (entryType) {
        case EntryType.grpc:
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees ParsedGrpcRequest
            return buildScriptGrpcRequest(request, grpcContext?.auth ?? null);
        default:
            // Header mutation (ADR-167) is HTTP/GraphQL only; gRPC has no header facade.
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees HTTP/GraphQL
            return buildScriptHttpRequest(request, collector);
    }
}
export function buildScriptResponse(response, libs, entryType) {
    switch (entryType) {
        case EntryType.grpc:
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees GrpcScriptResponseData
            return buildScriptGrpcResponse(response, libs);
        default:
            // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- entryType discriminant guarantees HTTP/GraphQL
            return buildScriptHttpResponse(response, libs);
    }
}
function createMessageAssertions(data, negate, libs) {
    function parse() {
        try {
            return { ok: true, value: JSON.parse(data) };
        }
        catch {
            return { ok: false, value: undefined };
        }
    }
    const be = {
        get json() {
            assertCondition(parse().ok, 'Expected message to be valid JSON', negate);
            return undefined;
        },
        get present() {
            assertCondition(data.length > 0, 'Expected message to be non-empty', negate);
            return undefined;
        },
    };
    const have = {
        body(expected) {
            assertCondition(data.includes(expected), `Expected message to include "${expected}"`, negate);
        },
        jsonBody(...args) {
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
            assertCondition(libs.lodash.isEqual(actual, expected), `Expected message path "${path}" to equal ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`, negate);
        },
    };
    return negate ? { be, have } : { be, have, not: createMessageAssertions(data, true, libs) };
}
/** Build the `rq.message` surface for one iteration of an on-message script. */
export function buildScriptMessage(message, libs) {
    const assertions = createMessageAssertions(message.data, false, libs);
    return {
        index: message.index,
        timestamp: message.timestamp,
        data: message.data,
        // `createMessageAssertions(_, false, _)` always returns the un-negated shape,
        // which carries `not`. Narrowed by construction rather than asserted.
        to: 'not' in assertions ? assertions : { ...assertions, not: assertions },
        json() {
            try {
                return JSON.parse(message.data);
            }
            catch (err) {
                throw new Error('Message is not valid JSON', { cause: err });
            }
        },
        text() {
            return message.data;
        },
        toJSON() {
            return { index: message.index, timestamp: message.timestamp, data: message.data };
        },
    };
}
