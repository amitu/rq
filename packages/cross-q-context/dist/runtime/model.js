// cross-q-context — the request/response DATA MODEL the rq.* API reads (self-contained).
//
// A clean-room model of the API-client request/response a script sees via `rq.request` /
// `rq.response`, plus the variable + realtime-message shapes. Requestly's app derives the
// equivalent from `@requestly/schemas` (zod-inferred); cross-q-context defines it here with ZERO
// app dependency, matching the app's field shapes so the rq.* API and the app's mapping layer line
// up at the seam. Deep auth is carried opaquely (`Json`). Realtime CONNECT/PUBLISH request shapes
// (MQTT/WS/Socket.IO) are deferred; the on-message + gRPC-stream response shapes are here.
// ── protocol enums ──────────────────────────────────────────────────────────────────────────
export var EntryType;
(function (EntryType) {
    EntryType["http"] = "http";
    EntryType["graphql"] = "graphql";
    EntryType["grpc"] = "grpc";
    EntryType["mqtt"] = "mqtt";
    EntryType["websocket"] = "websocket";
    EntryType["socketio"] = "socketio";
})(EntryType || (EntryType = {}));
export var RequestMethod;
(function (RequestMethod) {
    RequestMethod["GET"] = "GET";
    RequestMethod["POST"] = "POST";
    RequestMethod["PUT"] = "PUT";
    RequestMethod["PATCH"] = "PATCH";
    RequestMethod["DELETE"] = "DELETE";
    RequestMethod["HEAD"] = "HEAD";
    RequestMethod["OPTIONS"] = "OPTIONS";
})(RequestMethod || (RequestMethod = {}));
export var RequestContentType;
(function (RequestContentType) {
    RequestContentType["raw"] = "raw";
    RequestContentType["json"] = "json";
    RequestContentType["form"] = "form";
    RequestContentType["formData"] = "multipart/form-data";
    RequestContentType["binary"] = "binary";
    RequestContentType["none"] = "none";
})(RequestContentType || (RequestContentType = {}));
export var RawBodyContentType;
(function (RawBodyContentType) {
    RawBodyContentType["text"] = "text/plain";
    RawBodyContentType["json"] = "application/json";
    RawBodyContentType["html"] = "text/html";
    RawBodyContentType["xml"] = "application/xml";
    RawBodyContentType["javascript"] = "application/javascript";
})(RawBodyContentType || (RawBodyContentType = {}));
export var AuthType;
(function (AuthType) {
    AuthType["inherit"] = "inherit";
    AuthType["basicAuth"] = "basic_auth";
    AuthType["bearerToken"] = "bearer_token";
    AuthType["apiKey"] = "api_key";
    AuthType["oauth2"] = "oauth_2";
    AuthType["oauth1"] = "oauth_1";
    AuthType["jwtBearer"] = "jwt_bearer";
    AuthType["digestAuth"] = "digest_auth";
    AuthType["hawk"] = "hawk";
    AuthType["awsSigV4"] = "aws_sigv4";
    AuthType["ntlm"] = "ntlm";
})(AuthType || (AuthType = {}));
export var GrpcMethodType;
(function (GrpcMethodType) {
    GrpcMethodType["unary"] = "unary";
    GrpcMethodType["serverStreaming"] = "server_streaming";
    GrpcMethodType["clientStreaming"] = "client_streaming";
    GrpcMethodType["bidiStreaming"] = "bidi_streaming";
})(GrpcMethodType || (GrpcMethodType = {}));
// ── variables ───────────────────────────────────────────────────────────────────────────────
/** A variable's value type. Canonical enum (matches the app's) — the executor compares against
 * `secret` and maps raw mutation types onto these members. */
export var VariableDataType;
(function (VariableDataType) {
    VariableDataType["string"] = "string";
    VariableDataType["number"] = "number";
    VariableDataType["boolean"] = "boolean";
    VariableDataType["secret"] = "secret";
    VariableDataType["array"] = "array";
})(VariableDataType || (VariableDataType = {}));
