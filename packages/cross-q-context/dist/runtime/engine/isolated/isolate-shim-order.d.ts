/**
 * The in-isolate shim strings, eval'd in order inside the isolate after the host
 * callbacks are installed. Console is first (always wired), then the capability
 * shims. crypto must precede any consumer that calls `__rq_concatAB`.
 */
export declare const ISOLATE_SHIMS: readonly string[];
