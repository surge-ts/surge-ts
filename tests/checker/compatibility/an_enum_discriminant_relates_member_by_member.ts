// @strict: true
enum MyEnum { A = 1, B = 2 }
type TypeA = { kind: MyEnum.A, id?: number };
type TypeB = { kind: MyEnum.B } & ({ id?: undefined } | { id: number });
type MyType = TypeA | TypeB;
declare function something(a: MyType): void;
function indirect(kind: MyEnum, id?: number): void {
    something({ kind, id });
}
