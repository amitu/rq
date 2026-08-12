// cross-q-context — the request/response DATA MODEL the rq.* API reads (self-contained).
//
// A clean-room model of the API-client request/response the script sees via `rq.request` /
// `rq.response`. Requestly's app derives the equivalent from `@requestly/schemas` (zod-inferred);
// cross-q-context defines it here with ZERO app dependency, matching the app's field shapes so the
// rq.* API and the app's mapping layer line up at the seam. Deep auth is carried opaquely (`Json`)
// for now — the script rarely reshapes it, and the app maps its rich auth onto it at the boundary.
// Realtime protocols (MQTT / WebSocket / Socket.IO) are deferred until the rq.* API needs them.
/** The protocol of an entry. */
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
/** Top-level body content-type selector Requestly stores on a request. */
export var RequestContentType;
(function (RequestContentType) {
    RequestContentType["raw"] = "raw";
    RequestContentType["json"] = "json";
    RequestContentType["form"] = "form";
    RequestContentType["formData"] = "multipart/form-data";
    RequestContentType["binary"] = "binary";
    RequestContentType["none"] = "none";
})(RequestContentType || (RequestContentType = {}));
/** The editor language of a `raw` body — doubles as its Content-Type. */
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
