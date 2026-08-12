/* @ts-self-types="./cross_q_context.d.ts" */
import * as wasm from "./cross_q_context_bg.wasm";
import { __wbg_set_wasm } from "./cross_q_context_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    extract_requires, transform
} from "./cross_q_context_bg.js";
