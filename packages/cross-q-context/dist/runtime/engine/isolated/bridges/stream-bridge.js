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
export const STREAM_ISOLATE_SHIM = `
(() => {
  class EventEmitter {
    constructor() { this._listeners = {}; }
    on(evt, fn) { (this._listeners[evt] ||= []).push(fn); return this; }
    once(evt, fn) {
      const wrap = (...a) => { this.off(evt, wrap); fn(...a); };
      return this.on(evt, wrap);
    }
    off(evt, fn) {
      this._listeners[evt] = (this._listeners[evt] || []).filter((f) => f !== fn);
      return this;
    }
    emit(evt, ...args) {
      (this._listeners[evt] || []).slice().forEach((f) => f(...args));
      return (this._listeners[evt] || []).length > 0;
    }
  }
  class Readable extends EventEmitter {
    constructor() { super(); this._buf = []; }
    push(chunk) { if (chunk === null) { this.emit('end'); } else { this._buf.push(chunk); this.emit('data', chunk); } return true; }
    pipe(dest) { this.on('data', (c) => dest.write && dest.write(c)); this.on('end', () => dest.end && dest.end()); return dest; }
  }
  class Writable extends EventEmitter {
    constructor() { super(); this._chunks = []; }
    write(chunk) { this._chunks.push(chunk); this.emit('drain'); return true; }
    end(chunk) { if (chunk !== undefined) this.write(chunk); this.emit('finish'); }
  }
  class Duplex extends Readable {}
  class PassThrough extends Readable {
    write(chunk) { this.push(chunk); return true; }
    end() { this.push(null); }
  }
  globalThis.__rq_streamModule = { Readable, Writable, Duplex, PassThrough, Stream: Readable, EventEmitter };
})();
`;
