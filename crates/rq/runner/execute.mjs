// The host side of `rq`'s script execution: JSON in on stdin, JSON out on stdout.
//
// `rq` is a Rust binary and cross-q-context's engine is JavaScript driving QuickJS-on-WASM,
// so something has to bridge them. This is that something, and it is deliberately the
// smallest thing that can be: read one `ScriptExecutionInput`, hand it to `executeScript`,
// print the `ScriptExecutionResult`. No logic of its own — anything clever in here would be
// a second implementation of the runtime's semantics, which is the thing cross-q-context
// exists to prevent.
//
// Invoked as: node execute.mjs <path-to-cross-q-context>

import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const packageRoot = process.argv[2];
if (!packageRoot) {
  process.stderr.write('usage: execute.mjs <path-to-cross-q-context>\n');
  process.exit(2);
}

function fail(message) {
  // A failure here is the *host* failing, not the script failing — the difference matters,
  // so it goes to stderr and a non-zero exit rather than into the result.
  process.stderr.write(`${message}\n`);
  process.exit(1);
}

const entry = pathToFileURL(`${packageRoot}/dist/runtime/engine/execute.js`).href;
let executeScript;
try {
  ({ executeScript } = await import(entry));
} catch (error) {
  fail(
    `could not load the engine from ${entry}\n` +
      `  ${error?.message ?? error}\n` +
      '  is cross-q-context built, and are its dependencies installed?',
  );
}

let input;
try {
  input = JSON.parse(readFileSync(0, 'utf8'));
} catch (error) {
  fail(`the input was not JSON: ${error?.message ?? error}`);
}

try {
  const result = await executeScript({
    script: input.script,
    phase: input.phase,
    context: input.context,
    ...(input.timeoutMs !== undefined ? { timeoutMs: input.timeoutMs } : {}),
  });
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  // The engine threw rather than returning — still a result the run can carry, because a
  // script that blows up is a fact about the run, not a reason to lose it.
  process.stdout.write(
    JSON.stringify({
      mutationDiff: {},
      logs: [],
      testResults: [],
      error: String(error?.stack ?? error?.message ?? error),
    }),
  );
}
