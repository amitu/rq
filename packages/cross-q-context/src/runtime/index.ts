// The cross-q-context scripting runtime — public surface. Step 1 (ADR-213 Layer 2) ships the
// self-contained CONTRACT; the rq.* API and the QuickJS engine land on top of it next.
export * from './contract.js';
