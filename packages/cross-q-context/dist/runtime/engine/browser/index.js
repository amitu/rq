// cross-q-context — the BROWSER host for the safe QuickJS engine.
//
// The same QuickJsEngine desktop/node runs, wired to a browser QuickJsHostConfig: the browser WASM
// variant + browser-native capability backends (fflate for zlib, @noble/hashes for crypto,
// node-inspect-extracted for util) instead of node:*. A SEPARATE entry point so node consumers of
// the engine never pull the browser WASM variant. The web-worker RPC layer stays app-side — it just
// instantiates BrowserQuickJsSandbox.
export { BrowserQuickJsSandbox, createBrowserQuickJsHost, createBrowserRequireSupport } from './sandbox.js';
export { getQuickJsModule as getBrowserQuickJsModule } from './quickjs-module.js';
