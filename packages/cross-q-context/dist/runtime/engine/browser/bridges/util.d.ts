export type UtilOp = {
    readonly json: string;
};
export type UtilResult = {
    readonly text: string;
};
export declare function browserUtilHandler(req: UtilOp): UtilResult;
