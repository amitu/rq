/**
 * In-isolate shim for the Safe-mode `util` bridge.
 *
 * Lives here, not beside its host callback, because it is pure guest-realm JS
 * text with no host dependency — the half of the bridge that is identical on
 * every host. `@requestly/sandbox-node` re-exports it, so existing import sites
 * are unchanged. Keep this file free of imports.
 */

/**
 * In-isolate JS: builds the `util` subset. `format` is pure in-isolate (printf-ish
 * %s/%d/%j); `inspect` defers to the host bridge for fidelity.
 */
export const UTIL_ISOLATE_SHIM = `
(() => {
  const call = globalThis.__rq_util_inspect;
  const inspect = (value) => {
    try { return call({ json: JSON.stringify(value) }).text; }
    catch { return String(value); }
  };
  const format = (fmt, ...args) => {
    if (typeof fmt !== 'string') return [fmt, ...args].map((a) => inspect(a)).join(' ');
    let i = 0;
    let out = fmt.replace(/%[sdjifoO%]/g, (m) => {
      if (m === '%%') return '%';
      if (i >= args.length) return m;
      const a = args[i++];
      if (m === '%d' || m === '%i') return String(parseInt(a, 10));
      if (m === '%f') return String(parseFloat(a));
      if (m === '%j') { try { return JSON.stringify(a); } catch { return '[Circular]'; } }
      if (m === '%s') return String(a);
      return inspect(a);
    });
    for (; i < args.length; i++) out += ' ' + inspect(args[i]);
    return out;
  };
  globalThis.__rq_utilModule = { inspect, format };
})();
`;
