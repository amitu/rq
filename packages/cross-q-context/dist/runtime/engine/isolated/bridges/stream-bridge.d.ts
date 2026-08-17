/**
 * stream-bridge — Safe-mode `stream` stub (NEEDS_BRIDGE, ADR-010 §34).
 *
 * Many pure-JS packages `require('stream')` only to subclass or feature-detect
 * it, not to move bytes through a live OS stream. A live stream IS a host
 * resource and is out of scope (that is Developer mode / §5.2.3). This stub
 * provides the in-isolate class surface (Readable/Writable/PassThrough/Duplex)
 * with an in-memory buffer so those packages load — but it is NOT a live OS
 * stream. There is no host capability behind it, so there is no host callback:
 * the stub is pure in-isolate JS. It is registered as a "bridge" only so it
 * goes through the same install + containment-test discipline.
 *
 * HARD INVARIANT: trivially held — nothing crosses the isolate edge at all.
 */
/**
 * In-isolate JS: minimal EventEmitter-backed stream classes. Pure in-isolate;
 * no host call. Enough for require('stream') to succeed and for in-memory
 * PassThrough usage; file/socket streaming is IMPOSSIBLE (guided error elsewhere).
 */
export declare const STREAM_ISOLATE_SHIM = "\n(() => {\n  class EventEmitter {\n    constructor() { this._listeners = {}; }\n    on(evt, fn) { (this._listeners[evt] ||= []).push(fn); return this; }\n    once(evt, fn) {\n      const wrap = (...a) => { this.off(evt, wrap); fn(...a); };\n      return this.on(evt, wrap);\n    }\n    off(evt, fn) {\n      this._listeners[evt] = (this._listeners[evt] || []).filter((f) => f !== fn);\n      return this;\n    }\n    emit(evt, ...args) {\n      (this._listeners[evt] || []).slice().forEach((f) => f(...args));\n      return (this._listeners[evt] || []).length > 0;\n    }\n  }\n  class Readable extends EventEmitter {\n    constructor() { super(); this._buf = []; }\n    push(chunk) { if (chunk === null) { this.emit('end'); } else { this._buf.push(chunk); this.emit('data', chunk); } return true; }\n    pipe(dest) { this.on('data', (c) => dest.write && dest.write(c)); this.on('end', () => dest.end && dest.end()); return dest; }\n  }\n  class Writable extends EventEmitter {\n    constructor() { super(); this._chunks = []; }\n    write(chunk) { this._chunks.push(chunk); this.emit('drain'); return true; }\n    end(chunk) { if (chunk !== undefined) this.write(chunk); this.emit('finish'); }\n  }\n  class Duplex extends Readable {}\n  class PassThrough extends Readable {\n    write(chunk) { this.push(chunk); return true; }\n    end() { this.push(null); }\n  }\n  globalThis.__rq_streamModule = { Readable, Writable, Duplex, PassThrough, Stream: Readable, EventEmitter };\n})();\n";
