// @strict: true
interface IWithCallSignatures {
    (a: number): string;
}
interface IWithCallSignatures2 {
    (a: number): number;
}
interface IWithCallSignatures3 {
    (b: string): number;
}
var x2: IWithCallSignatures | IWithCallSignatures2 = a => a.toString();
var x3: IWithCallSignatures | IWithCallSignatures3 = a => a.toString();
