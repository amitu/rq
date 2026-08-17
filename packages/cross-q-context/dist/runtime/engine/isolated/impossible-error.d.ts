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
import type { ScriptPackageUnsupportedReason } from '../../index.js';
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
export declare function createImpossiblePackageError(packageId: string, reason: ScriptPackageUnsupportedReason, options?: {
    readonly cause?: unknown;
}): ScriptPackageUnsupportedError;
/**
 * Type-guard for the IMPOSSIBLE-tail error. The isolate engine uses this in its
 * catch to lift the typed `{ reason, packageId }` onto the result so the runtime
 * can carry it as `EntryError.details` (the `Script Package Unsupported`
 * analytics event consumes it on the client). Checks the sentinel plus the two
 * attached fields so a plain `PackageError` (no classification) is not matched.
 */
export { isScriptPackageUnsupportedError } from './package-error-sentinel.js';
