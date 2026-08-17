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
export declare const UTIL_ISOLATE_SHIM = "\n(() => {\n  const call = globalThis.__rq_util_inspect;\n  const inspect = (value) => {\n    try { return call({ json: JSON.stringify(value) }).text; }\n    catch { return String(value); }\n  };\n  const format = (fmt, ...args) => {\n    if (typeof fmt !== 'string') return [fmt, ...args].map((a) => inspect(a)).join(' ');\n    let i = 0;\n    let out = fmt.replace(/%[sdjifoO%]/g, (m) => {\n      if (m === '%%') return '%';\n      if (i >= args.length) return m;\n      const a = args[i++];\n      if (m === '%d' || m === '%i') return String(parseInt(a, 10));\n      if (m === '%f') return String(parseFloat(a));\n      if (m === '%j') { try { return JSON.stringify(a); } catch { return '[Circular]'; } }\n      if (m === '%s') return String(a);\n      return inspect(a);\n    });\n    for (; i < args.length; i++) out += ' ' + inspect(args[i]);\n    return out;\n  };\n  globalThis.__rq_utilModule = { inspect, format };\n})();\n";
