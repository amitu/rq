/**
 * In-isolate shim for the Safe-mode `buffer` bridge.
 *
 * Lives here, not beside its host callback, because it is pure guest-realm JS
 * text with no host dependency — the half of the bridge that is identical on
 * every host. `@requestly/sandbox-node` re-exports it, so existing import sites
 * are unchanged. Keep this file free of imports.
 */
/**
 * In-isolate JS: builds a minimal `Buffer` global on top of `__rq_buffer`.
 * Evaluated inside the isolate (never run host-side). Presents `Buffer.from`,
 * `.alloc`/`.allocUnsafe`, `.concat`, `.isBuffer`, `.toString(enc)`, and length —
 * the data-shaped subset common npm packages reach for (uuid, nanoid, crypto-js).
 * Allocation methods run ENTIRELY in-isolate (a `Uint8Array` of the given size);
 * no host call is needed, so nothing crosses the edge for them. Anything beyond
 * this subset is undefined inside the isolate (Developer mode is the escape hatch).
 */
export const BUFFER_ISOLATE_SHIM = `
(() => {
  const call = globalThis.__rq_buffer;
  class SafeBuffer extends Uint8Array {
    static from(input, encoding) {
      if (input instanceof Uint8Array) return new SafeBuffer(input);
      if (Array.isArray(input)) return new SafeBuffer(Uint8Array.from(input));
      const enc = encoding || 'utf8';
      const res = call({ op: 'encode', input: String(input), encoding: enc });
      // bytes cross as an ArrayBuffer; rebuild the realm's own Uint8Array view.
      return new SafeBuffer(new Uint8Array(res.bytes));
    }
    // alloc/allocUnsafe are pure in-isolate allocations (no host round-trip).
    // Node zero-fills alloc(); allocUnsafe() may contain old memory, but a fresh
    // Uint8Array is already zero-filled, which is a safe superset of the contract.
    static alloc(size, fill) {
      const b = new SafeBuffer(size >>> 0);
      if (fill !== undefined && fill !== 0) b.fill(typeof fill === 'number' ? fill : 0);
      return b;
    }
    static allocUnsafe(size) {
      return new SafeBuffer(size >>> 0);
    }
    static isBuffer(obj) {
      return obj instanceof SafeBuffer;
    }
    static concat(list, totalLength) {
      let len = totalLength;
      if (len === undefined) { len = 0; for (const b of list) len += b.length; }
      const out = new SafeBuffer(len >>> 0);
      let offset = 0;
      for (const b of list) {
        if (offset >= out.length) break;
        out.set(b.subarray(0, Math.min(b.length, out.length - offset)), offset);
        offset += b.length;
      }
      return out;
    }
    toString(encoding) {
      const enc = encoding || 'utf8';
      // Send bytes out as an ArrayBuffer (sliced to this view's exact window).
      const res = call({ op: 'decode', bytes: this.buffer.slice(this.byteOffset, this.byteOffset + this.byteLength), encoding: enc });
      return res.text;
    }
  }
  globalThis.Buffer = SafeBuffer;
})();
`;
