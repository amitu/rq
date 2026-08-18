// Which entry points are allowed to touch node:*, and which are not.
//
// The rule is not decorative. `browser/sandbox.ts` carried a comment saying the guest realm
// must be reached only by Node-free paths — and imported it from a barrel that also exports
// NodeSandbox, whose static `import * as vm from 'node:vm'` rode along into the browser
// graph. A bundler either fails on that or silently ships a polyfill. It came back twice
// while being fixed (engine.ts and browser/bridges/fetch.ts each re-entered the same barrel),
// which is exactly why it is asserted here instead of remembered.
import { readFileSync, existsSync } from 'node:fs';
import { dirname, resolve, relative } from 'node:path';

const HERE = dirname(new URL(import.meta.url).pathname);
const D = (p) => resolve(HERE, '..', p);

// Entries a browser or a non-Node host loads: nothing from node: may be statically reachable.
const NODE_FREE = [
  ['runtime (types + rq.* definitions)', D('dist/runtime/index.js')],
  ['executor/browser', D('dist/runtime/engine/browser/index.js')],
  // rq's CLI loads this one directly. It runs under Node today, but it must stay free of
  // node:vm specifically: the Developer engine is a host-embedding for trusted code, and a
  // CLI running a collection's scripts is not that.
  ['engine/execute (rq loads this)', D('dist/runtime/engine/execute.js')],
];

function reach(entry, target) {
  const seen = new Map();
  const hits = [];
  const walk = (file, parent) => {
    if (seen.has(file)) return;
    seen.set(file, parent);
    const src = readFileSync(file, 'utf8');
    const specs = [];
    for (const m of src.matchAll(/(?:^|\n)\s*(?:import|export)[^;\n]*?from\s*['"]([^'"]+)['"]/g)) specs.push(m[1]);
    for (const m of src.matchAll(/\bimport\(\s*['"]([^'"]+)['"]\s*\)/g)) specs.push(m[1]);
    for (const spec of specs) {
      if (spec === target || (target === 'node:*' && (spec.startsWith('node:') || ['fs', 'path', 'vm', 'crypto', 'zlib'].includes(spec)))) {
        const chain = [];
        for (let c = file; c; c = seen.get(c)) chain.unshift(relative(D('.'), c));
        hits.push({ spec, chain });
        continue;
      }
      if (!spec.startsWith('.')) continue;
      const p = resolve(dirname(file), spec);
      if (existsSync(p)) walk(p, file);
    }
  };
  walk(entry, null);
  return hits;
}

let failed = 0;
for (const [name, entry] of NODE_FREE) {
  const target = name.startsWith('engine/execute') ? 'node:vm' : 'node:*';
  const hits = reach(entry, target);
  if (hits.length === 0) {
    console.log(`  ✓ ${name}: no static ${target}`);
  } else {
    failed += 1;
    console.log(`  ✗ ${name}: reaches ${target} —`);
    for (const h of hits.slice(0, 2)) console.log(`      ${h.spec} via ${h.chain.join(' → ')}`);
    console.log(`      import from the defining module, not from a barrel that re-exports the Node engine`);
  }
}

// The counterpart: the Node executor entry SHOULD carry the Developer engine. If this ever
// passes, NodeSandbox stopped being reachable where requestly-api-client expects it.
const devReach = reach(D('dist/runtime/engine/index.js'), 'node:vm');
if (devReach.length > 0) console.log('  ✓ executor (Node entry): still ships the node:vm Developer engine');
else { failed += 1; console.log('  ✗ executor (Node entry): NodeSandbox is no longer reachable — requestly-api-client depends on it'); }

if (failed) { console.error(`\nEntry hygiene FAILED (${failed})`); process.exit(1); }
console.log('\nEntry hygiene OK — node:* stays on the Node entries.');
