// @strict: true
type Inferred<P> = { [K in keyof P]: P[K] extends Checker<infer T, infer R> ? Checker<T, R>[typeof checkedType] : {} };
declare const checkedType: unique symbol;
interface Checker<U, Required = false> {
    (props: any): boolean;
    isRequired: Checker<U, true>;
    [checkedType]: Required extends true ? U : U | null | undefined;
}
declare const numberChecker: Checker<number>;
declare const stringChecker: Checker<string>;
const shape = { count: numberChecker, label: stringChecker.isRequired };
declare const inferred: Inferred<typeof shape>;
const count: number | null | undefined = inferred.count;
const label: string = inferred.label;
const wrong: boolean = inferred.label;
