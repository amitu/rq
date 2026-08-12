import { parseRequireId } from './parse-require-id';

import type { InstallPackageSpec } from './install-types';

export type DuplicatePackageError =
  | { readonly kind: 'version-conflict'; readonly packageName: string; readonly specifiers: readonly string[] }
  | { readonly kind: 'unversioned-vs-versioned'; readonly packageName: string; readonly specifiers: readonly string[] };

export interface ExtractedRequire {
  readonly rawId: string;
}

export type ValidateRequiresResult =
  | { readonly ok: true; readonly value: readonly InstallPackageSpec[] }
  | { readonly ok: false; readonly error: DuplicatePackageError };

export function validateRequires(requires: readonly ExtractedRequire[]): ValidateRequiresResult {
  const groups = new Map<string, string[]>();

  for (const req of requires) {
    const parsed = parseRequireId(req.rawId);
    const existing = groups.get(parsed.packageName);
    if (existing) {
      existing.push(req.rawId);
    } else {
      groups.set(parsed.packageName, [req.rawId]);
    }
  }

  for (const [packageName, specifiers] of groups) {
    if (specifiers.length <= 1) continue;

    const uniqueVersions = [...new Set(specifiers.map((s) => parseRequireId(s).version ?? '__unversioned__'))];
    if (uniqueVersions.length <= 1) continue;

    const unique = [...new Set(specifiers)];
    const hasVersioned = uniqueVersions.some((v) => v !== '__unversioned__');
    const hasUnversioned = uniqueVersions.includes('__unversioned__');

    const kind = hasVersioned && hasUnversioned ? 'unversioned-vs-versioned' : 'version-conflict';
    return { ok: false, error: { kind, packageName, specifiers: unique } };
  }

  const specs: InstallPackageSpec[] = [];
  const seen = new Set<string>();

  for (const req of requires) {
    const parsed = parseRequireId(req.rawId);
    if (seen.has(parsed.packageName)) continue;
    seen.add(parsed.packageName);
    specs.push({ packageName: parsed.packageName, version: parsed.version });
  }

  return { ok: true, value: specs };
}
