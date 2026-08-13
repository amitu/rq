import type { Json } from './contract.js';
export declare enum EntryType {
    http = "http",
    graphql = "graphql",
    grpc = "grpc",
    mqtt = "mqtt",
    websocket = "websocket",
    socketio = "socketio"
}
export declare enum RequestMethod {
    GET = "GET",
    POST = "POST",
    PUT = "PUT",
    PATCH = "PATCH",
    DELETE = "DELETE",
    HEAD = "HEAD",
    OPTIONS = "OPTIONS"
}
export declare enum RequestContentType {
    raw = "raw",
    json = "json",
    form = "form",
    formData = "multipart/form-data",
    binary = "binary",
    none = "none"
}
export declare enum RawBodyContentType {
    text = "text/plain",
    json = "application/json",
    html = "text/html",
    xml = "application/xml",
    javascript = "application/javascript"
}
export declare enum AuthType {
    inherit = "inherit",
    basicAuth = "basic_auth",
    bearerToken = "bearer_token",
    apiKey = "api_key",
    oauth2 = "oauth_2",
    oauth1 = "oauth_1",
    jwtBearer = "jwt_bearer",
    digestAuth = "digest_auth",
    hawk = "hawk",
    awsSigV4 = "aws_sigv4",
    ntlm = "ntlm"
}
export declare enum GrpcMethodType {
    unary = "unary",
    serverStreaming = "server_streaming",
    clientStreaming = "client_streaming",
    bidiStreaming = "bidi_streaming"
}
/** A key/value pair at the runtime boundary — UI metadata (id/isEnabled/description) already stripped. */
export interface KeyValue {
    key: string;
    value: string;
}
/** Alias matching the app's boundary name. */
export type ParsedKeyValue = KeyValue;
export interface FormDataKeyValue {
    key: string;
    value: string;
    type: string;
}
export interface PathVariable {
    key: string;
    value: string;
}
export interface HttpBody {
    contentType: string;
    raw?: string;
    rawContentType?: string;
    formUrlEncoded: KeyValue[];
    formData: FormDataKeyValue[];
    binary?: {
        name: string;
        path: string;
    };
}
export interface HttpRequest {
    url: string;
    method: string;
    headers: KeyValue[];
    queryParams: KeyValue[];
    pathVariables: PathVariable[];
    body: HttpBody;
    contentType: string;
    auth?: Json;
}
export type ParsedHttpRequest = HttpRequest;
export interface HttpResponse {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: string;
    time: number;
    size: number;
    /** Body byte encoding (ADR-153); absent ⇒ 'utf8'. `string` for seam compatibility. */
    bodyEncoding?: string;
}
export interface GraphQLBody {
    query: string;
    variables?: string;
    operationName?: string;
}
export interface GraphQLRequest {
    url: string;
    method: string;
    headers: KeyValue[];
    queryParams: KeyValue[];
    body: GraphQLBody;
    /** The operation query string (top-level for the script facade's `extractBody`). */
    query: string;
    auth?: Json;
}
export type ParsedGraphQLRequest = GraphQLRequest;
export interface GraphQLResponse {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: string;
    time: number;
    size: number;
}
export interface GrpcRequest {
    url: string;
    methodPath: string;
    metadata: KeyValue[];
    /** The serialized request message (JSON text). */
    message: string;
    auth?: Json;
}
export type ParsedGrpcRequest = GrpcRequest;
export interface GrpcStreamMessage {
    readonly data: string;
    readonly timestamp: number;
}
export interface GrpcScriptResponse {
    statusCode: number;
    statusMessage: string;
    metadata: Record<string, string>;
    trailers: Record<string, string>;
    messages: GrpcStreamMessage[];
    responseTime: number;
}
export interface ScriptMessageInput {
    readonly index: number;
    readonly data: string;
    readonly timestamp: number;
}
/** The variable value type. `string` (not a literal union) so the app's nominal `VariableDataType`
 * enum — string/number/boolean/secret/array — assigns at the seam; consumers switch on the value. */
export type VariableDataType = string;
/** A resolved variable at the runtime boundary (the app's `VariableData`). */
export interface VariableData {
    localValue: string;
    syncValue: string;
    type: VariableDataType;
    id?: string;
    isPersisted?: boolean;
    isEnabled?: boolean;
}
export type EnvironmentVariables = Record<string, VariableData>;
