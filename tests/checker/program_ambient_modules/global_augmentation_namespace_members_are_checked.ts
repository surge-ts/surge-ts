namespace N { export interface I {} }
declare global {
    namespace G {
        type U = N.Missing;
        type W = N.I;
        interface Box { value: N.Gone; }
    }
}
export {};
