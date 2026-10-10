// `checkTypeArgumentConstraints` reports at the argument that violates its
// parameter's constraint, a setter's parameter type included.
type Fail<T extends never> = T;
type Num<T extends number> = T;
interface I1 {
    get x(): number;
    set x(value: Fail<string>);
}
interface I2 {
    y(value: Num<boolean>): void;
}
interface I3 {
    z: Num<string>;
}
class C1 {
    set x(value: Fail<number>) {}
}
let v: Num<"a">;
