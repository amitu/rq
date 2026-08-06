/* @ts-self-types="./cross_q_engine.d.ts" */
import * as wasm from "./cross_q_engine_bg.wasm";
import { __wbg_set_wasm } from "./cross_q_engine_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    formats, parse, version
} from "./cross_q_engine_bg.js";
