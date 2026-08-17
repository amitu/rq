/**
 * impossible-error — the IMPOSSIBLE-tail guided error (ADR-010 §77/§87, FR-06/FR-16).
 *
 * A package that needs a live OS socket, live fd, native `.node` addon, or
 * asymmetric crypto cannot run in Safe mode. Instead of a silent `undefined` or
 * a cryptic stack, the require chain throws THIS error: it names the limitation
 * and a concrete next step, and carries a bounded `ScriptPackageUnsupportedReason`
 * as a typed field so Slice 3's `Script Package Unsupported` analytics event can
 * classify the failure machine-readably.
 *
 * The error reuses `vm-package-evaluator`'s `PackageError` sentinel so the
 * require chain passes it through already-attributed (not re-wrapped).
 *
 * `gr-static-error-messages`: the message is a bounded composition over a closed
 * set of reasons — not a free-form interpolation of arbitrary runtime values.
 */

import { createPackageError } from './package-error-sentinel.js';

import type { ScriptPackageUnsupportedReason } from '../../index.js';

/**
 * The guided next-step copy per reason — a closed map, so the message stays a
 * bounded static string (the package id is the only interpolated value, and it
 * comes from the require id the user typed).
 */
const GUIDANCE: Readonly<Record<ScriptPackageUnsupportedReason, string>> = {
  native_addon:
    'it needs a native (.node) addon, which Safe mode cannot load. Switch this request to Developer mode to use it.',
  live_socket:
    'it needs a live network socket, which Safe mode does not expose. Use the built-in fetch, or switch this request to Developer mode.',
  live_fs:
    'it needs live filesystem access, which Safe mode does not expose. Switch this request to Developer mode to use it.',
  asymmetric_crypto:
    'asymmetric crypto (RS256/ES/PS) is not available in Safe mode. Use HS256, or switch this request to Developer mode.',
  other: 'it is not supported in Safe mode. Switch this request to Developer mode to use it.',
};

/**
 * An error thrown when an IMPOSSIBLE package is required in Safe mode. Carries
 * the package id and the bounded reason for downstream (Slice 3) analytics.
 */
export type { ScriptPackageUnsupportedError } from './package-error-sentinel.js';

import type { ScriptPackageUnsupportedError } from './package-error-sentinel.js';

/**
 * Build the guided IMPOSSIBLE-tail error. The require chain throws this for any
 * package classified `impossible`.
 *
 * `cause` preserves the underlying failure (e.g. the esbuild error when an
 * installed package can't be bundled) per `gr-preserve-cause-chains`. The
 * user-facing message stays the bounded guided string; the cause rides along for
 * diagnostics without leaking a raw stack to the script.
 */
export function createImpossiblePackageError(
  packageId: string,
  reason: ScriptPackageUnsupportedReason,
  options?: { readonly cause?: unknown },
): ScriptPackageUnsupportedError {
  // Static, bounded message: `packageId` is the user-supplied require id; the
  // explanatory clause is selected from the closed GUIDANCE map.
  const err =
    options?.cause === undefined
      ? createPackageError(`Package '${packageId}' cannot be used in Safe mode — ${GUIDANCE[reason]}`)
      : createPackageError(`Package '${packageId}' cannot be used in Safe mode — ${GUIDANCE[reason]}`, {
          cause: options.cause,
        });
  // Attach the typed classification. The base PackageError already carries the
  // sentinel, so the require chain passes it through unchanged.
  return Object.assign(err, {
    unsupportedReason: reason,
    packageId,
  }) as ScriptPackageUnsupportedError;
}

/**
 * Type-guard for the IMPOSSIBLE-tail error. The isolate engine uses this in its
 * catch to lift the typed `{ reason, packageId }` onto the result so the runtime
 * can carry it as `EntryError.details` (the `Script Package Unsupported`
 * analytics event consumes it on the client). Checks the sentinel plus the two
 * attached fields so a plain `PackageError` (no classification) is not matched.
 */
export { isScriptPackageUnsupportedError } from './package-error-sentinel.js';
