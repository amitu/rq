import { parseRequireId } from './parse-require-id.js';
export function validateRequires(requires) {
    const groups = new Map();
    for (const req of requires) {
        const parsed = parseRequireId(req.rawId);
        const existing = groups.get(parsed.packageName);
        if (existing) {
            existing.push(req.rawId);
        }
        else {
            groups.set(parsed.packageName, [req.rawId]);
        }
    }
    for (const [packageName, specifiers] of groups) {
        if (specifiers.length <= 1)
            continue;
        const uniqueVersions = [...new Set(specifiers.map((s) => parseRequireId(s).version ?? '__unversioned__'))];
        if (uniqueVersions.length <= 1)
            continue;
        const unique = [...new Set(specifiers)];
        const hasVersioned = uniqueVersions.some((v) => v !== '__unversioned__');
        const hasUnversioned = uniqueVersions.includes('__unversioned__');
        const kind = hasVersioned && hasUnversioned ? 'unversioned-vs-versioned' : 'version-conflict';
        return { ok: false, error: { kind, packageName, specifiers: unique } };
    }
    const specs = [];
    const seen = new Set();
    for (const req of requires) {
        const parsed = parseRequireId(req.rawId);
        if (seen.has(parsed.packageName))
            continue;
        seen.add(parsed.packageName);
        specs.push({ packageName: parsed.packageName, version: parsed.version });
    }
    return { ok: true, value: specs };
}
