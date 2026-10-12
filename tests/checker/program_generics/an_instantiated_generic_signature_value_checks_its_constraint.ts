type Test<T> = <T1 extends T>(data: T1) => T1;
declare const test: Test<number>;
test(1);
test('hi');
declare function outer<T>(t: T): <V extends T>(v: V) => V;
const o = outer(1);
o(2);
o('s');
