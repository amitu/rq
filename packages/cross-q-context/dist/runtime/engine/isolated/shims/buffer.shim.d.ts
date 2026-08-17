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
export declare const BUFFER_ISOLATE_SHIM = "\n(() => {\n  const call = globalThis.__rq_buffer;\n  class SafeBuffer extends Uint8Array {\n    static from(input, encoding) {\n      if (input instanceof Uint8Array) return new SafeBuffer(input);\n      if (Array.isArray(input)) return new SafeBuffer(Uint8Array.from(input));\n      const enc = encoding || 'utf8';\n      const res = call({ op: 'encode', input: String(input), encoding: enc });\n      // bytes cross as an ArrayBuffer; rebuild the realm's own Uint8Array view.\n      return new SafeBuffer(new Uint8Array(res.bytes));\n    }\n    // alloc/allocUnsafe are pure in-isolate allocations (no host round-trip).\n    // Node zero-fills alloc(); allocUnsafe() may contain old memory, but a fresh\n    // Uint8Array is already zero-filled, which is a safe superset of the contract.\n    static alloc(size, fill) {\n      const b = new SafeBuffer(size >>> 0);\n      if (fill !== undefined && fill !== 0) b.fill(typeof fill === 'number' ? fill : 0);\n      return b;\n    }\n    static allocUnsafe(size) {\n      return new SafeBuffer(size >>> 0);\n    }\n    static isBuffer(obj) {\n      return obj instanceof SafeBuffer;\n    }\n    static concat(list, totalLength) {\n      let len = totalLength;\n      if (len === undefined) { len = 0; for (const b of list) len += b.length; }\n      const out = new SafeBuffer(len >>> 0);\n      let offset = 0;\n      for (const b of list) {\n        if (offset >= out.length) break;\n        out.set(b.subarray(0, Math.min(b.length, out.length - offset)), offset);\n        offset += b.length;\n      }\n      return out;\n    }\n    toString(encoding) {\n      const enc = encoding || 'utf8';\n      // Send bytes out as an ArrayBuffer (sliced to this view's exact window).\n      const res = call({ op: 'decode', bytes: this.buffer.slice(this.byteOffset, this.byteOffset + this.byteLength), encoding: enc });\n      return res.text;\n    }\n  }\n  globalThis.Buffer = SafeBuffer;\n})();\n";
