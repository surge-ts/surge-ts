// @filename: observable.ts
export declare class Observable<T> {
    filter(pred: (e: T) => boolean): Observable<T>;
}
// @filename: x0.ts
export let a = 1;
// @filename: x.ts
import { Observable } from "./observable";
declare module "./observable" {
    export * from "./x0";
    export { a } from "./x0";
}
export {};
