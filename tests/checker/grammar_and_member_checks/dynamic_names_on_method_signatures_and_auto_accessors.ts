function foo<T>() { return "" }
interface I<T> {
    bar(): string;
    [foo<T>()](): void;
}
var v: {
    ["" + ""](): void;
};
declare var f: any;
class C2 {
    accessor [f()] = 1;
}
