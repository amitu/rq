/**
 * classify-bundle-failure — map an esbuild bundle failure to a bounded
 * `ScriptPackageUnsupportedReason` (sandbox-node ADR-010 §87).
 *
 * Failure-driven, NOT a metadata pre-scan (ADR-010 §44): we inspect the esbuild
 * error that actually occurred rather than scanning package.json up front. The
 * dominant real blocker is a native `.node` addon — esbuild has no loader for
 * `.node` files and reports that in its `BuildFailure.errors[].text`. Everything
 * we can't positively identify stays `other` (no worse than today's blanket
 * `other`, and never a misleading precise claim).
 *
 * The thrown value is `unknown`; narrow it with guards + `Reflect.get`, never an
 * `as` cast (`gr-no-unsafe-cast`).
 */

import type { ScriptPackageUnsupportedReason } from '../../index.js';

/**
 * Collect the text of every esbuild error message on a thrown value. esbuild's
 * `BuildFailure` shape is `{ errors: Array<{ text: string, ... }> }`; we read it
 * structurally so a non-esbuild error simply yields no signatures.
 */
function esbuildErrorTexts(err: unknown): string[] {
  if (err === null || typeof err !== 'object') return [];
  const errors = Reflect.get(err, 'errors');
  if (!Array.isArray(errors)) return [];
  const texts: string[] = [];
  for (const e of errors) {
    if (e !== null && typeof e === 'object') {
      const text = Reflect.get(e, 'text');
      if (typeof text === 'string') texts.push(text);
    }
  }
  return texts;
}

/**
 * Map a caught esbuild bundle failure to a precise reason. Conservative: only the
 * stable native-`.node`-loader signature is positively classified; all else is
 * `other`. The matched substring is esbuild's "No loader is configured for
 * \".node\" files" message (also covers the generic ".node" mention in the path).
 */
export function classifyBundleFailure(err: unknown): ScriptPackageUnsupportedReason {
  const joined = esbuildErrorTexts(err).join('\n');
  // Native addon: esbuild can't inline a `.node` binary. The canonical message is
  // `No loader is configured for ".node" files`. Match the `.node` extension AND a
  // loader complaint together so an unrelated mention of ".node" alone doesn't
  // misclassify.
  if (/\.node\b/.test(joined) && /loader/i.test(joined)) {
    return 'native_addon';
  }
  return 'other';
}
