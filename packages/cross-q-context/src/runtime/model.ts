// cross-q-context — the request/response DATA MODEL the rq.* API reads (self-contained).
//
// A clean-room model of the API-client request/response a script sees via `rq.request` /
// `rq.response`, plus the variable + realtime-message shapes. Requestly's app derives the
// equivalent from `@requestly/schemas` (zod-inferred); cross-q-context defines it here with ZERO
// app dependency, matching the app's field shapes so the rq.* API and the app's mapping layer line
// up at the seam. Deep auth is carried opaquely (`Json`). Realtime CONNECT/PUBLISH request shapes
// (MQTT/WS/Socket.IO) are deferred; the on-message + gRPC-stream response shapes are here.

import type { Json } from './contract.js';

// ── protocol enums ──────────────────────────────────────────────────────────────────────────
export enum EntryType {
  http = 'http',
  graphql = 'graphql',
  grpc = 'grpc',
  mqtt = 'mqtt',
  websocket = 'websocket',
  socketio = 'socketio',
}

export enum RequestMethod {
  GET = 'GET',
  POST = 'POST',
  PUT = 'PUT',
  PATCH = 'PATCH',
  DELETE = 'DELETE',
  HEAD = 'HEAD',
  OPTIONS = 'OPTIONS',
}

export enum RequestContentType {
  raw = 'raw',
  json = 'json',
  form = 'form',
  formData = 'multipart/form-data',
  binary = 'binary',
  none = 'none',
}

export enum RawBodyContentType {
  text = 'text/plain',
  json = 'application/json',
  html = 'text/html',
  xml = 'application/xml',
  javascript = 'application/javascript',
}

export enum AuthType {
  inherit = 'inherit',
  basicAuth = 'basic_auth',
  bearerToken = 'bearer_token',
  apiKey = 'api_key',
  oauth2 = 'oauth_2',
  oauth1 = 'oauth_1',
  jwtBearer = 'jwt_bearer',
  digestAuth = 'digest_auth',
  hawk = 'hawk',
  awsSigV4 = 'aws_sigv4',
  ntlm = 'ntlm',
}

export enum GrpcMethodType {
  unary = 'unary',
  serverStreaming = 'server_streaming',
  clientStreaming = 'client_streaming',
  bidiStreaming = 'bidi_streaming',
}

// ── key/value + body ────────────────────────────────────────────────────────────────────────
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
  type: 'text' | 'file';
}

export interface PathVariable {
  key: string;
  value: string;
}

export interface HttpBody {
  contentType: RequestContentType;
  raw?: string;
  rawContentType?: RawBodyContentType;
  formUrlEncoded: KeyValue[];
  formData: FormDataKeyValue[];
  binary?: { name: string; path: string };
}

// ── HTTP request/response ───────────────────────────────────────────────────────────────────
export interface HttpRequest {
  url: string;
  method: RequestMethod;
  headers: KeyValue[];
  queryParams: KeyValue[];
  pathVariables: PathVariable[];
  body: HttpBody;
  contentType: RequestContentType;
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
  /** Body byte encoding (ADR-153); absent ⇒ 'utf8'. */
  bodyEncoding?: 'utf8' | 'base64';
}

// ── GraphQL request/response ────────────────────────────────────────────────────────────────
export interface GraphQLBody {
  query: string;
  variables?: string;
  operationName?: string;
}
export interface GraphQLRequest {
  url: string;
  method: RequestMethod;
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

// ── gRPC request/response (script-facing shapes) ────────────────────────────────────────────
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

// ── realtime on-message input ───────────────────────────────────────────────────────────────
export interface ScriptMessageInput {
  readonly index: number;
  readonly data: string;
  readonly timestamp: number;
}

// ── variables ───────────────────────────────────────────────────────────────────────────────
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
