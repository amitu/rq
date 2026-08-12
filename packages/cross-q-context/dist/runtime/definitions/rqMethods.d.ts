import { EntryType } from './_deps.js';
import type { ExecutionDirective, RequestHeaderMutation, ScriptExecutionContext, ScriptPhase, TestResult, VisualizerDirective } from './_deps.js';
import type { CookieJarBridge } from './cookies.js';
import type { AssertionLibs } from './requestResponse.js';
import type { RunRequestImpl } from './runRequest.js';
export interface RawMutationEntry {
    value: string;
    type: 'string' | 'number' | 'boolean' | 'array';
}
export type RawScopeMutations = {
    global?: Record<string, RawMutationEntry | null>;
    environment?: Record<string, RawMutationEntry | null>;
    collection?: Record<string, RawMutationEntry | null>;
    runtime?: Record<string, RawMutationEntry | null>;
};
/**
 * Creates the `rq` namespace object.
 *
 * This is the single source of truth for the rq scripting API.
 * Adding a new rq method = add it here, return it in the object.
 *
 * Takes VM dependencies as parameters — no `declare const`, no globals.
 * Works in Node.js, web workers, and browsers.
 */
export declare function createRqNamespace(executionState: {
    testResults: TestResult[];
    rawMutations: RawScopeMutations;
    requestMutations?: RequestHeaderMutation[];
    executionDirective?: ExecutionDirective;
    visualizerOutput?: VisualizerDirective;
}, libs: AssertionLibs, context: ScriptExecutionContext, eventName: ScriptPhase, cookieBridge?: CookieJarBridge, entryType?: EntryType, fetchImpl?: typeof globalThis.fetch, runRequestImpl?: RunRequestImpl): {
    test: {
        (name: string, testFn: () => void): void;
        skip(name: string, testFn?: () => void): void;
    };
    expect: Chai.ExpectStatic;
    info: Readonly<{
        requestId: string;
        requestName: string;
        iteration: number;
        iterationCount: number;
        entryIndex: number;
        totalEntries: number;
        eventName: ScriptPhase;
    }>;
    environment: {
        get(key: string): any;
        set(key: string, value: string | number | boolean | unknown[] | null | undefined): void;
        unset(key: string): void;
        clear(): void;
        has(key: string): boolean;
        /**
         * Serialization view of the scope — always string values, intentionally
         * asymmetric with `get()`. `get("n")` restores the recorded type (e.g. a
         * number), whereas `toObject().n` is the raw stored string. `toObject` is
         * for bulk inspection/serialization, where stringified values are wanted.
         */
        toObject(): Record<string, string>;
    };
    globals: {
        get(key: string): any;
        set(key: string, value: string | number | boolean | unknown[] | null | undefined): void;
        unset(key: string): void;
        clear(): void;
        has(key: string): boolean;
        /**
         * Serialization view of the scope — always string values, intentionally
         * asymmetric with `get()`. `get("n")` restores the recorded type (e.g. a
         * number), whereas `toObject().n` is the raw stored string. `toObject` is
         * for bulk inspection/serialization, where stringified values are wanted.
         */
        toObject(): Record<string, string>;
    };
    collectionVariables: {
        get(key: string): any;
        set(key: string, value: string | number | boolean | unknown[] | null | undefined): void;
        unset(key: string): void;
        clear(): void;
        has(key: string): boolean;
        /**
         * Serialization view of the scope — always string values, intentionally
         * asymmetric with `get()`. `get("n")` restores the recorded type (e.g. a
         * number), whereas `toObject().n` is the raw stored string. `toObject` is
         * for bulk inspection/serialization, where stringified values are wanted.
         */
        toObject(): Record<string, string>;
    };
    variables: {
        get(key: string): any;
        set(key: string, value: string | number | boolean | unknown[] | null | undefined): void;
        unset(key: string): void;
        clear(): void;
        has(key: string): boolean;
        /**
         * Serialization view of the scope — always string values, intentionally
         * asymmetric with `get()`. `get("n")` restores the recorded type (e.g. a
         * number), whereas `toObject().n` is the raw stored string. `toObject` is
         * for bulk inspection/serialization, where stringified values are wanted.
         */
        toObject(): Record<string, string>;
    };
    iterationData: {
        get(key: string): any;
        has(key: string): boolean;
        toObject(): Record<string, string>;
    };
    request: import("./requestResponse.js").ScriptRequest;
    response: import("./requestResponse.js").ScriptResponse | null;
    vault: {
        get(key: string): any;
        has(key: string): boolean;
        toObject(): Record<string, string>;
    };
    cookies: import("./cookies.js").ScriptCookiesNamespace;
    sendRequest: import("./sendRequest.js").ScriptSendRequest;
    execution: import("./execution.js").RqExecutionNamespace & {
        skipRequest?: () => never;
    };
    visualizer: import("./visualizer.js").RqVisualizerNamespace;
    message: import("./requestResponse.js").ScriptMessage | null;
    /** Whether this script is running in Safe mode (QuickJS-WASM) vs Developer mode (node:vm). */
    isSafeMode: boolean;
};
