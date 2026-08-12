// cross-q-context — the request/response DATA MODEL the rq.* API reads (self-contained).
//
// A clean-room model of the API-client request/response the script sees via `rq.request` /
// `rq.response`. Requestly's app derives the equivalent from `@requestly/schemas` (zod-inferred);
// cross-q-context defines it here with ZERO app dependency, matching the app's field shapes so the
// rq.* API and the app's mapping layer line up at the seam. Deep auth is carried opaquely (`Json`)
// for now — the script rarely reshapes it, and the app maps its rich auth onto it at the boundary.
// Realtime protocols (MQTT / WebSocket / Socket.IO) are deferred until the rq.* API needs them.

import type { Json } from './contract.js';

/** The protocol of an entry. */
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

/** Top-level body content-type selector Requestly stores on a request. */
export enum RequestContentType {
  raw = 'raw',
  json = 'json',
  form = 'form',
  formData = 'multipart/form-data',
  binary = 'binary',
  none = 'none',
}

/** The editor language of a `raw` body — doubles as its Content-Type. */
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

/** A key/value pair at the runtime boundary — UI metadata (id/isEnabled/description) already stripped. */
export interface KeyValue {
  key: string;
  value: string;
}

/** A form-data pair, which additionally carries whether the value is a text field or a file. */
export interface FormDataKeyValue {
  key: string;
  value: string;
  type: 'text' | 'file';
}

/** A path variable (`:id`). */
export interface PathVariable {
  key: string;
  value: string;
}

/** A request body at the runtime boundary. Array fields are always present (empty when unused). */
export interface HttpBody {
  contentType: RequestContentType;
  /** Present for a `raw` body. */
  raw?: string;
  /** The editor language of a `raw` body. */
  rawContentType?: RawBodyContentType;
  formUrlEncoded: KeyValue[];
  formData: FormDataKeyValue[];
  /** Reference to a binary body file, when `contentType === binary`. */
  binary?: { name: string; path: string };
}

/** An HTTP request the script reads via `rq.request`. Auth is opaque here (see file header). */
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

/** An HTTP response the script reads via `rq.response`. */
export interface HttpResponse {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  body: string;
  /** Round-trip time in milliseconds. */
  time: number;
}

/** A gRPC request the script reads via `rq.request` on a gRPC entry. */
export interface GrpcRequest {
  url: string;
  service: string;
  method: string;
  methodType: GrpcMethodType;
  metadata: KeyValue[];
  message: Json;
  auth?: Json;
}

/** A single gRPC stream message. */
export interface GrpcStreamMessage {
  payload: Json;
  metadata?: Record<string, string>;
}

/** A gRPC response the script reads via `rq.response`. */
export interface GrpcResponse {
  statusCode: number;
  statusText: string;
  metadata: Record<string, string>;
  messages: GrpcStreamMessage[];
}
