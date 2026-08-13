#!/usr/bin/env node
/**
 * CLI command for generating editor type declarations.
 *
 * Usage: generate-sandbox-types --output <path>
 *
 * Generates:
 * - primitives.d.ts — bundled ES2022 stdlib chain
 * - globals.d.ts — curated subset from lib.webworker.d.ts
 * - rq.pre-request.d.ts — rq namespace types (declare global)
 * - rq.post-response.d.ts — rq namespace types (phase-filtered)
 * - rq.dynamic.d.ts — typed $-prefixed dynamic variable signatures (ADR-055)
 *
 * The consumer provides the output location. This package has zero
 * knowledge of where the types end up.
 */

import { createRequire } from 'node:module';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import * as ts from 'typescript';

import { DEPRECATED_IDENTIFIERS } from '../deprecated-identifiers.js';
import { PHASE_RESTRICTED } from '../types.js';
import { EDITOR_TYPE_NAMES } from './globals-list.js';
import { EXTERNAL_BUILTIN_PACKAGES, NODE_BUILTIN_PACKAGES } from '../builtInPackages/index.js';
// The dynamic-variable METADATA (name/label/args/example) the editor autocompletes. Vendored as a
// self-contained snapshot — cross-q-context owns editor-type generation with ZERO app dependency
// (the faker value-generation stays host-injected at runtime; the codegen needs only the metadata).
import { DYNAMIC_VARIABLE_CATALOG } from './dynamic-variables-catalog.js';

import type { DeprecatedIdentifierPolicy } from '../deprecated-identifiers.js';
import type { ExternalBuiltinPackage } from '../builtInPackages/index.js';

// cross-q-context's own ScriptPhase — the codegen imports nothing from the app.
import { ScriptPhase } from '../../contract.js';

// ─── CLI args ───────────────────────────────────────────────

function parseArgs(): { outputDir: string } {
  const args = process.argv.slice(2);
  const outputIdx = args.indexOf('--output');
  if (outputIdx === -1 || !args[outputIdx + 1]) {
    // eslint-disable-next-line no-console -- CLI output
    console.error('Usage: generate-sandbox-types --output <path>');
    process.exit(1);
  }
  // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- validated above
  return { outputDir: path.resolve(args[outputIdx + 1]!) };
}

// ─── Paths ──────────────────────────────────────────────────

// This file lives at <pkg>/src/runtime/definitions/codegen/ — four levels below the package root.
// Resolve relative to it so paths hold whether run from the local worktree or a consumer's
// node_modules copy (the compiled rqMethods.d.ts ships in the SAME package's dist).
const HERE = path.dirname(fileURLToPath(import.meta.url));
const PKG_ROOT = path.resolve(HERE, '..', '..', '..', '..');
const RQ_METHODS_DTS = path.join(PKG_ROOT, 'dist', 'runtime', 'definitions', 'rqMethods.d.ts');

// typescript is a devDependency of this package but is NOT installed for consumers of a git/npm
// dep; resolve it from wherever the RUNNING process finds it (the app that invokes this codegen),
// robust to pnpm's nested layout — `require.resolve('typescript')` → <ts>/lib/typescript.js.
const TS_LIB_DIR = path.dirname(createRequire(import.meta.url).resolve('typescript'));
const WEBWORKER_DTS = path.join(TS_LIB_DIR, 'lib.webworker.d.ts');

const GLOBALS_TO_EXTRACT = new Set<string>(EDITOR_TYPE_NAMES);

// ─── Extract rq return type from rqMethods.d.ts ────────────

/**
 * Union type alias → preferred variant mapping per protocol.
 * When generating per-protocol `.d.ts`, union aliases resolve to the named variant
 * instead of defaulting to the first union member (HTTP).
 */
const GRPC_UNION_OVERRIDES: Record<string, string> = {
  ScriptRequest: 'ScriptGrpcRequest',
  ScriptResponse: 'ScriptGrpcResponse',
};

/**
 * Extracts exported and non-exported interface bodies from a .d.ts file.
 * Returns a map of interface name → inline type literal (e.g., `{ readonly url: string; ... }`).
 * Handles interfaces that reference other interfaces in the same file by recursively inlining.
 *
 * @param unionOverrides — when provided, union type aliases matching a key resolve to the
 *   named variant instead of the first union member. Used for per-protocol `.d.ts` generation.
 */
/**
 * A node's own text prefixed with its leading JSDoc/comment blocks (which `getText()` omits).
 * The comments are the trivia between the previous token and this node's start.
 */
function withLeadingComments(sourceFile: ts.SourceFile, node: ts.Node): string {
  const full = sourceFile.text;
  const ranges = ts.getLeadingCommentRanges(full, node.getFullStart()) ?? [];
  const comments = ranges.map((r) => full.slice(r.pos, r.end)).join('\n    ');
  const text = node.getText(sourceFile);
  return comments ? `${comments}\n    ${text}` : text;
}

function extractInterfaceBodies(filePath: string, unionOverrides?: Record<string, string>): Map<string, string> {
  const content = fs.readFileSync(filePath, 'utf-8');
  const sourceFile = ts.createSourceFile(filePath, content, ts.ScriptTarget.Latest, true);

  const interfaces = new Map<string, string>();

  ts.forEachChild(sourceFile, (node) => {
    if (ts.isInterfaceDeclaration(node)) {
      // Preserve each member's leading JSDoc — the whole point of the editor types is hover-docs.
      // `m.getText()` drops leading comments; splice them back from the source's comment ranges.
      const members = node.members.map((m) => withLeadingComments(sourceFile, m)).join('\n    ');
      interfaces.set(node.name.text, `{\n    ${members}\n  }`);
    }
    if (ts.isTypeAliasDeclaration(node) && ts.isUnionTypeNode(node.type)) {
      const aliasName = node.name.text;
      const overrideTarget = unionOverrides?.[aliasName];

      if (overrideTarget) {
        // Per-protocol override: resolve to the specified variant
        interfaces.set(aliasName, `__DEFERRED_REF_${overrideTarget}__`);
      } else {
        // Default: resolve to the first variant (HTTP) for the default .d.ts.
        const firstType = node.type.types[0];
        if (firstType && ts.isTypeReferenceNode(firstType) && ts.isIdentifier(firstType.typeName)) {
          const refName = firstType.typeName.text;
          interfaces.set(aliasName, `__DEFERRED_REF_${refName}__`);
        }
      }
    }
  });

  // Resolve deferred union type references — point to the actual interface body.
  for (const [name, body] of interfaces) {
    const deferredMatch = body.match(/^__DEFERRED_REF_(\w+)__$/);
    if (deferredMatch?.[1]) {
      const targetBody = interfaces.get(deferredMatch[1]);
      if (targetBody) {
        interfaces.set(name, targetBody);
      }
    }
  }

  // Resolve cross-references between interfaces within the same file.
  // E.g., ScriptResponse references ResponseAssertions, which references StatusAssertions/HaveAssertions.
  // Do multiple passes to handle transitive references.
  const MAX_PASSES = 5;
  for (let pass = 0; pass < MAX_PASSES; pass++) {
    let changed = false;
    for (const [name, body] of interfaces) {
      // Look for bare type references that match other interfaces in this file
      let resolved = body;
      for (const [refName, refBody] of interfaces) {
        if (refName === name) continue;
        // Match standalone type references (as property types) — word boundary ensures we
        // don't replace partial matches. The reference appears after `:` in a property.
        const refPattern = new RegExp(`:\\s*${refName}\\b(?!\\.)`, 'g');
        if (refPattern.test(resolved)) {
          resolved = resolved.replace(new RegExp(`(:\\s*)${refName}\\b(?!\\.)`, 'g'), `$1${refBody}`);
          changed = true;
        }
      }
      interfaces.set(name, resolved);
    }
    if (!changed) break;
  }

  return interfaces;
}

function extractRqReturnType(filePath: string, unionOverrides?: Record<string, string>): string {
  const content = fs.readFileSync(filePath, 'utf-8');
  const sourceFile = ts.createSourceFile(filePath, content, ts.ScriptTarget.Latest, true);

  // Find createRqNamespace function and extract its return type members
  let returnTypeText = '';

  ts.forEachChild(sourceFile, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'createRqNamespace') {
      const returnType = node.type;
      if (returnType) {
        returnTypeText = returnType.getText(sourceFile);
      }
    }
  });

  // If no explicit return type, try to infer from the function body
  // The tsc output should have an explicit return type in the .d.ts
  if (!returnTypeText) {
    // Fallback: parse the return type from the full text
    const match = content.match(/createRqNamespace\([^)]*\):\s*(\{[^}]+\})/s);
    if (match?.[1]) {
      returnTypeText = match[1];
    }
  }

  // Resolve import("./<sibling>").TypeName references by inlining the interface
  // bodies from sibling .d.ts files. Keeps the generated .d.ts self-contained so
  // the editor sandbox doesn't need to resolve imports.
  // cross-q-context splits the rq.* surface across these modules; the return type references them
  // via `import("./<module>.js").<Type>`. Inline each so the editor .d.ts stays self-contained.
  // NodeNext emits the `.js` extension in the .d.ts — the pattern tolerates it (optional).
  const SIBLING_MODULES = ['requestResponse', 'cookies', 'sendRequest', 'execution', 'visualizer'];
  for (const moduleName of SIBLING_MODULES) {
    const importRefPattern = new RegExp(`import\\("\\./${moduleName}(?:\\.js)?"\\)\\.(\\w+)`, 'g');
    const referencedTypes = new Set<string>();
    let importMatch;
    while ((importMatch = importRefPattern.exec(returnTypeText)) !== null) {
      if (importMatch[1]) {
        referencedTypes.add(importMatch[1]);
      }
    }
    if (referencedTypes.size === 0) continue;

    const siblingDts = path.join(path.dirname(filePath), `${moduleName}.d.ts`);
    const interfaceBodies = extractInterfaceBodies(siblingDts, unionOverrides);

    for (const typeName of referencedTypes) {
      const inlinedBody = interfaceBodies.get(typeName);
      if (!inlinedBody) {
        throw new Error('Could not find interface in sibling .d.ts for inlining', {
          cause: { typeName, moduleName },
        });
      }
      returnTypeText = returnTypeText.replace(
        new RegExp(`import\\("\\./${moduleName}(?:\\.js)?"\\)\\.${typeName}`, 'g'),
        inlinedBody,
      );
    }
  }

  return returnTypeText;
}

/**
 * Removes a named member from a TypeScript type literal string using the AST.
 * Handles arbitrarily nested types (e.g., `response: { to: { be: { ok: void; }; }; }`).
 */
function removeTypeMember(typeStr: string, memberName: string): string {
  const wrapper = `type __RqFiltered = ${typeStr}`;
  const sourceFile = ts.createSourceFile('__filter.ts', wrapper, ts.ScriptTarget.Latest, true);

  // Find the type alias declaration
  const typeAlias = sourceFile.statements[0];
  if (!typeAlias || !ts.isTypeAliasDeclaration(typeAlias) || !ts.isTypeLiteralNode(typeAlias.type)) {
    return typeStr;
  }

  const typeLiteral = typeAlias.type;
  const memberToRemove = typeLiteral.members.find(
    (m) => ts.isPropertySignature(m) && m.name && ts.isIdentifier(m.name) && m.name.text === memberName,
  );

  if (!memberToRemove) return typeStr;

  // Calculate positions relative to the original typeStr (offset by "type __RqFiltered = " prefix)
  const prefixLength = wrapper.indexOf(typeStr);
  const memberStart = memberToRemove.getFullStart() - prefixLength;
  const memberEnd = memberToRemove.getEnd() - prefixLength;

  return typeStr.slice(0, memberStart) + typeStr.slice(memberEnd);
}

function generateRqDts(returnType: string, phase?: ScriptPhase): string {
  // Filter out phase-restricted members using AST-based removal.
  // The regex approach ([^;]+;) breaks on nested types with inner semicolons.
  // When phase is undefined, skip filtering — produce a superset with all members (ADR-091).
  let filteredType = returnType;
  if (phase !== undefined) {
    for (const [entryName, allowedPhases] of Object.entries(PHASE_RESTRICTED)) {
      if (allowedPhases && !allowedPhases.includes(phase)) {
        filteredType = removeTypeMember(filteredType, entryName);
      }
    }
  }

  // Resolve external type references to inline literals so the generated
  // .d.ts is self-contained (no imports needed in the editor sandbox).
  // ScriptPhase enum → string literal union.
  const scriptPhaseValues = Object.values(ScriptPhase)
    .map((v) => `"${v}"`)
    .join(' | ');
  filteredType = filteredType.replace(/\bScriptPhase\b/g, scriptPhaseValues);

  // GrpcStreamMessage (from @requestly/shared-types/runtime) → inline object type.
  filteredType = filteredType.replace(
    /\bGrpcStreamMessage\b/g,
    '{ readonly data: string; readonly timestamp: number }',
  );

  // rq.execution (ADR-169) — tsc renders the namespace as an import() reference
  // intersected with the conditional skipRequest:
  //   `import("./execution").RqExecutionNamespace & { skipRequest?: () => never }`
  // Inline the WHOLE intersection to a self-contained object literal (the editor
  // sandbox can't resolve import() refs). `skipRequest` is PHASE-GATED: present
  // only pre-request (Postman parity — it `is not a function` in post-response),
  // and in the all-phases superset (phase === undefined). `runRequest` is
  // optional (engine-gated). The replaced span includes the trailing
  // `& { skipRequest?: () => never }` so it does not leak into post-response.
  const includeSkipRequest = phase === undefined || phase === ScriptPhase.preRequest;
  const executionMembers = [
    '{',
    '    /** Set the next request to run by name (collection runner); null stops the run. */',
    '    setNextRequest(nameOrNull: string | null): void;',
    '    /** Ordered folder path [collection, ...folders, request]; `.current` is the request name. */',
    '    location: readonly string[] & { readonly current: string | undefined };',
    '    /** Run a saved request by its persistent ID and await its response (only when the host wires it). */',
    '    runRequest?: (requestId: string, opts?: { variableOverrides?: Record<string, string> }) => Promise<{ code: number; status: string; headers: Record<string, string>; responseBody: string; responseTime: number }>;',
    ...(includeSkipRequest
      ? [
          '    /** Skip the current request — pre-request only; aborts the rest of the pre-request script. */',
          '    skipRequest(): void;',
        ]
      : []),
    '  }',
  ].join('\n');
  filteredType = filteredType.replace(
    /import\("[^"]*\/execution"\)\.RqExecutionNamespace\s*&\s*\{[^}]*\}/g,
    executionMembers,
  );

  // rq.visualizer (ADR-202) — tsc renders the namespace as an import() reference
  // (`import("./visualizer").RqVisualizerNamespace`) the editor sandbox cannot
  // resolve; inline it to a self-contained object literal. `data` is typed
  // JsonValue in the source (gr-no-any); the editor surface widens it to `unknown`
  // so the generated .d.ts stays self-contained (no shared-types import). Present in
  // BOTH the pre-request and post-response .d.ts — `rq.visualizer` is available in both
  // phases (ADR-202 "Amendment (2026-08-02)"; no longer in PHASE_RESTRICTED).
  const visualizerMembers = [
    '{',
    '    /** Render a Handlebars template with optional data as the response visualization. Available in both pre-request and post-response scripts (Postman parity); last-writer-wins across the chain. */',
    '    set(template: string, data?: unknown): void;',
    '    /** Clear the current visualization. */',
    '    clear(): void;',
    '  }',
  ].join('\n');
  filteredType = filteredType.replace(/import\("[^"]*\/visualizer"\)\.RqVisualizerNamespace/g, visualizerMembers);

  // Exclude collectionId from rq.info — internal only, not user-facing (ADR-053).
  // TODO: This regex is fragile — it pattern-matches tsc output format. If tsc changes
  // whitespace or ordering, the match silently fails. PR #738 replaces this with
  // AST-based member filtering via the TypeScript compiler API.
  filteredType = filteredType.replace(/\s*collectionId:\s*string\s*\|\s*null;\s*/g, '\n');

  return `// Auto-generated by @requestly/cross-q-context — do not edit.
// Source: @requestly/cross-q-context (createRqNamespace return type)
export {};
declare global {
  interface RqNamespace ${filteredType}
  const rq: RqNamespace;
}
`;
}

// ─── Extract curated globals from lib.webworker.d.ts ────────

function getDeclarationName(node: ts.Node): string | undefined {
  if (ts.isInterfaceDeclaration(node)) return node.name.text;
  if (ts.isTypeAliasDeclaration(node)) return node.name.text;
  if (ts.isVariableStatement(node)) {
    const decl = node.declarationList.declarations[0];
    if (decl && ts.isIdentifier(decl.name)) return decl.name.text;
  }
  if (ts.isFunctionDeclaration(node) && node.name) return node.name.text;
  return undefined;
}

function collectReferencedTypeNames(node: ts.Node): Set<string> {
  const names = new Set<string>();
  function visit(n: ts.Node): void {
    if (ts.isHeritageClause(n)) {
      n.types.forEach((t) => {
        if (ts.isIdentifier(t.expression)) names.add(t.expression.text);
      });
    }
    if (ts.isTypeReferenceNode(n) && ts.isIdentifier(n.typeName)) {
      names.add(n.typeName.text);
    }
    ts.forEachChild(n, visit);
  }
  visit(node);
  return names;
}

function collectDeclaredNames(content: string): Set<string> {
  const sf = ts.createSourceFile('lib.d.ts', content, ts.ScriptTarget.Latest, true);
  const names = new Set<string>();
  ts.forEachChild(sf, (node) => {
    const name = getDeclarationName(node);
    if (name) names.add(name);
  });
  return names;
}

function extractGlobals(filePath: string, primitivesNames: Set<string>): string {
  const content = fs.readFileSync(filePath, 'utf-8');
  const sourceFile = ts.createSourceFile(filePath, content, ts.ScriptTarget.Latest, true);

  const declsByName = new Map<string, ts.Node[]>();
  ts.forEachChild(sourceFile, (node) => {
    const name = getDeclarationName(node);
    if (name) {
      const existing = declsByName.get(name);
      if (existing) {
        existing.push(node);
      } else {
        declsByName.set(name, [node]);
      }
    }
  });

  const needed = new Set<string>(GLOBALS_TO_EXTRACT);
  const MAX_DEPTH = 3;
  for (let depth = 0; depth < MAX_DEPTH; depth++) {
    for (const name of [...needed]) {
      const nodes = declsByName.get(name);
      if (!nodes) continue;
      for (const node of nodes) {
        for (const ref of collectReferencedTypeNames(node)) {
          if (!needed.has(ref) && declsByName.has(ref)) needed.add(ref);
        }
      }
    }
  }

  const unresolved = new Set<string>();
  for (const name of needed) {
    const nodes = declsByName.get(name);
    if (!nodes) continue;
    for (const node of nodes) {
      for (const ref of collectReferencedTypeNames(node)) {
        if (!needed.has(ref) && !primitivesNames.has(ref) && ref.length > 1) {
          unresolved.add(ref);
        }
      }
    }
  }

  const chunks: string[] = [];
  chunks.push('// Auto-generated by @requestly/cross-q-context — do not edit.');
  chunks.push('// Source: TypeScript lib.webworker.d.ts');

  if (unresolved.size > 0) {
    const stubs = [...unresolved]
      .sort()
      .map((name) => `interface ${name} {}`)
      .join('\n');
    chunks.push(`// Stub declarations for transitive dependencies\n${stubs}`);
  }

  ts.forEachChild(sourceFile, (node) => {
    const name = getDeclarationName(node);
    if (name && needed.has(name)) {
      chunks.push(node.getText(sourceFile));
    }
  });

  return chunks.join('\n\n');
}

// ─── Bundle ES2022 lib chain ────────────────────────────────

function bundleLibChain(entryLib: string): string {
  const visited = new Set<string>();
  const chunks: string[] = [];

  function visit(libFile: string): void {
    const resolved = path.join(TS_LIB_DIR, libFile);
    if (visited.has(resolved)) return;
    visited.add(resolved);
    if (!fs.existsSync(resolved)) return;

    const content = fs.readFileSync(resolved, 'utf-8');
    const refPattern = /\/\/\/\s*<reference\s+lib="([^"]+)"\s*\/>/g;
    let match;
    while ((match = refPattern.exec(content)) !== null) {
      visit(`lib.${match[1]}.d.ts`);
    }

    const stripped = content.replace(/\/\/\/\s*<reference[^>]*\/>\s*\n?/g, '').trim();
    if (stripped.length > 0) {
      chunks.push(`// --- ${libFile} ---\n${stripped}`);
    }
  }

  visit(entryLib);
  return `// Auto-generated by @requestly/cross-q-context — do not edit.\n// Source: TypeScript ES2022 stdlib\n${chunks.join('\n\n')}`;
}

// ─── Generate require.d.ts ───────────────────────────────────

function generateRequireDts(): string {
  // typeof import() references are resolved by the editor's ATA (Automatic Type
  // Acquisition) which fetches package .d.ts files from CDN and injects them into
  // the VFS. Before ATA completes, TS2307 ("Cannot find module") fires transiently —
  // the editor suppresses this diagnostic code.
  // Widen from the `as const` tuple to the interface so the optional `internal`
  // field is visible on every element (mirrors isolated-require.ts's buildClassMap).
  const externals: readonly ExternalBuiltinPackage[] = EXTERNAL_BUILTIN_PACKAGES;
  const overloads = externals
    // Internal (vendor-only) packages ship the IIFE into both guests but are NOT
    // user-requirable, so they get no require() overload / autocomplete / Packages
    // dropdown entry (ADR-202 Decision 2 — e.g. Handlebars).
    .filter((pkg) => !pkg.internal)
    .map(
      // id = what users type in require() (e.g., 'csv-parse/lib/sync')
      // entry = what npm resolves for types (e.g., 'csv-parse/sync')
      (pkg) => `declare function require(id: '${pkg.id}'): typeof import('${pkg.entry}');`,
    );
  // Node built-ins — types resolved from @types/node in the VFS
  for (const pkg of NODE_BUILTIN_PACKAGES) {
    overloads.push(`declare function require(id: '${pkg.id}'): typeof import('${pkg.id}');`);
  }
  // User-authored custom packages — .js extension discriminator (ADR-087, ADR-091)
  overloads.push('declare function require(id: `${string}.js`): any;');
  overloads.push('declare function require(id: string): unknown;');

  return `// Auto-generated by @requestly/cross-q-context — do not edit.
// Source: @requestly/cross-q-context (builtInPackages registry)
${overloads.join('\n')}
`;
}

// ─── Generate rq.dynamic.d.ts (ADR-055) ─────────────────────

function generateDynamicVariablesDts(): string {
  // The vendored metadata snapshot — cross-q-context owns this catalog (no app import).
  const variables = DYNAMIC_VARIABLE_CATALOG;

  const signatures: string[] = [];

  for (const meta of variables) {
    // JSDoc
    const jsdocLines = [`    /**`, `     * ${meta.label}`, `     * ${meta.description}`];
    if (meta.args) {
      const sorted = [...meta.args].sort((a, b) => a.order - b.order);
      for (const arg of sorted) {
        const optMark = arg.optional ? ' (optional)' : '';
        const defVal = arg.defaultValue ? ` — default: ${arg.defaultValue}` : '';
        jsdocLines.push(`     * @param ${arg.name} ${arg.description}${optMark}${defVal}`);
      }
    }
    jsdocLines.push(`     * @example ${JSON.stringify(meta.example)}`);
    jsdocLines.push(`     */`);

    // Function signature
    let params = '';
    if (meta.args && meta.args.length > 0) {
      const sorted = [...meta.args].sort((a, b) => a.order - b.order);
      params = sorted.map((arg) => `${arg.name}${arg.optional !== false ? '?' : ''}: ${arg.type}`).join(', ');
    }

    signatures.push(`${jsdocLines.join('\n')}\n    ${meta.name}(${params}): string | number | boolean;`);
  }

  return `// Auto-generated by @requestly/cross-q-context — do not edit.
// Source: @requestly/variables DynamicVariableResolver.list()
// Variable count: ${variables.length}
export {};
declare global {
  // Extends RqNamespace declared in rq.pre-request.d.ts / rq.post-response.d.ts.
  // TypeScript merges interface declarations, adding $-prefixed methods to rq.
  interface RqNamespace {
${signatures.join('\n\n')}
  }
}
`;
}

// ─── Convenience globals for globals.d.ts ────────────────────

const CONVENIENCE_GLOBALS = `
// Convenience globals for built-in packages
declare const _: typeof import('lodash');
declare function xml2Json(xmlString: string): unknown;
declare const CryptoJS: typeof import('crypto-js');
`;

// ─── Deprecated Postman identifiers for globals.d.ts ─────────
// Declared so the editor shows them as @deprecated (strikethrough + hover)
// instead of a hard "Cannot find name" error. They execute at runtime via the
// Slice C shims (ADR-156) / warn-and-no-op (ADR-155 RQ-3464); the editor surface
// must discourage them without erroring. Generated from DEPRECATED_IDENTIFIERS
// so it never drifts from the runtime warning registry.

/** Editor-facing TS type for each deprecated identifier (loose where we only discourage). */
const DEPRECATED_GLOBAL_TYPES: Record<string, string> = {
  // Shimmed core (ADR-156) — typed to match the runtime shim's delegation target.
  globals:
    '{ get(key: string): any; set(key: string, value: any): void; unset(key: string): void; has(key: string): boolean; toObject(): Record<string, string>; [key: string]: any }',
  environment:
    '{ get(key: string): any; set(key: string, value: any): void; unset(key: string): void; has(key: string): boolean; toObject(): Record<string, string>; [key: string]: any }',
  responseBody: 'string',
  responseCode: '{ code: number; [key: string]: any }',
  // Warn-and-no-op / rewrite-only (loose — we only discourage them).
  responseHeaders: 'any',
  responseCookies: 'any',
  responseTime: 'number',
  iteration: 'number',
  tests: 'Record<string, any>',
  data: 'any',
  request: 'any',
  tv4: 'any',
  Backbone: 'any',
};

function deprecationHint(policy: DeprecatedIdentifierPolicy): string {
  if (policy.kind === 'warn-and-suggest-rq') return `Use ${policy.replacement} instead.`;
  return policy.alternative === null
    ? 'This identifier is not supported in Requestly.'
    : `Use ${policy.alternative} instead.`;
}

function generateDeprecatedGlobalsDts(): string {
  const lines = ['', '// Deprecated Postman identifiers (discouraged — see ADR-155 / ADR-156)'];
  for (const [name, policy] of Object.entries(DEPRECATED_IDENTIFIERS)) {
    const tsType = DEPRECATED_GLOBAL_TYPES[name] ?? 'any';
    lines.push(`/** @deprecated ${deprecationHint(policy)} */`);
    lines.push(`declare const ${name}: ${tsType};`);
  }
  return lines.join('\n') + '\n';
}

// ─── Main ───────────────────────────────────────────────────

/**
 * Generate the editor type-declaration set into `outputDir`. The public entry — a consumer
 * (the app's `generate:sandbox-types` step) imports this and owns where the `.d.ts` land;
 * cross-q-context owns HOW they're generated and has zero knowledge of the destination.
 */
export function generateEditorTypes(outputDir: string): void {
  if (!fs.existsSync(RQ_METHODS_DTS)) {
    throw new Error('rqMethods.d.ts not found. Build @requestly/cross-q-context first (pnpm build:runtime).');
  }
  if (!fs.existsSync(WEBWORKER_DTS)) {
    throw new Error('lib.webworker.d.ts not found. Is typescript installed?');
  }

  // 1. rq namespace types — from createRqNamespace return type
  const returnType = extractRqReturnType(RQ_METHODS_DTS);
  if (!returnType) {
    throw new Error('Could not extract return type from createRqNamespace in rqMethods.d.ts');
  }
  const rqPreRequestDts = generateRqDts(returnType, ScriptPhase.preRequest);
  const rqPostResponseDts = generateRqDts(returnType, ScriptPhase.postResponse);
  const rqPackageDts = generateRqDts(returnType); // superset — no phase filtering (ADR-091)

  // 1b. gRPC per-protocol .d.ts (ADR-136 §6) — union types resolve to gRPC variants
  const grpcReturnType = extractRqReturnType(RQ_METHODS_DTS, GRPC_UNION_OVERRIDES);
  if (!grpcReturnType) {
    throw new Error('Could not extract gRPC return type from createRqNamespace in rqMethods.d.ts');
  }
  const rqGrpcPreRequestDts = generateRqDts(grpcReturnType, ScriptPhase.preRequest);
  const rqGrpcPostResponseDts = generateRqDts(grpcReturnType, ScriptPhase.postResponse);
  // ADR-208: on-message editor types. gRPC-only today — on-message is a streaming
  // hook and gRPC is the only protocol wired to it — so there is no
  // protocol-neutral `rq.on-message.d.ts` peer to the two above.
  const rqGrpcOnMessageDts = generateRqDts(grpcReturnType, ScriptPhase.onMessage);

  // 2. Primitives — bundled ES2022 stdlib
  const primitivesDts = bundleLibChain('lib.es2022.d.ts');
  const primitivesNames = collectDeclaredNames(primitivesDts);

  // 3. Globals — curated subset from lib.webworker.d.ts + convenience globals
  const globalsDts =
    extractGlobals(WEBWORKER_DTS, primitivesNames) + CONVENIENCE_GLOBALS + generateDeprecatedGlobalsDts();

  // 4. require() overloads — from EXTERNAL_BUILTIN_PACKAGES and NODE_BUILTIN_PACKAGES registries
  const requireDts = generateRequireDts();

  // 5. Dynamic variables — ADR-055: typed $-prefixed method signatures
  const dynamicVariablesDts = generateDynamicVariablesDts();

  // Write to output directory
  fs.mkdirSync(outputDir, { recursive: true });
  fs.writeFileSync(path.join(outputDir, 'rq.pre-request.d.ts'), rqPreRequestDts);
  fs.writeFileSync(path.join(outputDir, 'rq.post-response.d.ts'), rqPostResponseDts);
  fs.writeFileSync(path.join(outputDir, 'rq.package.d.ts'), rqPackageDts);
  fs.writeFileSync(path.join(outputDir, 'rq.grpc.pre-request.d.ts'), rqGrpcPreRequestDts);
  fs.writeFileSync(path.join(outputDir, 'rq.grpc.post-response.d.ts'), rqGrpcPostResponseDts);
  fs.writeFileSync(path.join(outputDir, 'rq.grpc.on-message.d.ts'), rqGrpcOnMessageDts);
  fs.writeFileSync(path.join(outputDir, 'primitives.d.ts'), primitivesDts);
  fs.writeFileSync(path.join(outputDir, 'globals.d.ts'), globalsDts);
  fs.writeFileSync(path.join(outputDir, 'require.d.ts'), requireDts);
  fs.writeFileSync(path.join(outputDir, 'rq.dynamic.d.ts'), dynamicVariablesDts);

  // eslint-disable-next-line no-console -- CLI output
  console.log(`Generated sandbox editor types in ${outputDir}:`);
  // eslint-disable-next-line no-console -- CLI output
  console.log(`  - rq.pre-request.d.ts`);
  // eslint-disable-next-line no-console -- CLI output
  console.log(`  - rq.post-response.d.ts`);
  process.stdout.write('  - rq.package.d.ts (superset — all phases, ADR-091)\n');
  process.stdout.write('  - rq.grpc.pre-request.d.ts (gRPC protocol, ADR-136)\n');
  process.stdout.write('  - rq.grpc.on-message.d.ts (gRPC streaming on-message, ADR-208)\n');
  process.stdout.write('  - rq.grpc.post-response.d.ts (gRPC protocol, ADR-136)\n');
  // eslint-disable-next-line no-console -- CLI output
  console.log(`  - primitives.d.ts (ES2022 stdlib, ${primitivesDts.split('\n').length} lines)`);
  // eslint-disable-next-line no-console -- CLI output
  console.log(`  - globals.d.ts`);
  // eslint-disable-next-line no-console -- CLI output
  console.log(
    `  - require.d.ts (${EXTERNAL_BUILTIN_PACKAGES.length} registry packages [internal entries excluded from overloads] + ${NODE_BUILTIN_PACKAGES.length} Node built-in overloads)`,
  );
  // eslint-disable-next-line no-console -- CLI output
  console.log(`  - rq.dynamic.d.ts (${dynamicVariablesDts.split('\n').length} lines)`);
}

// CLI entry — only when run directly (`tsx generate-types.ts --output <dir>`), not when imported.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  generateEditorTypes(parseArgs().outputDir);
}
