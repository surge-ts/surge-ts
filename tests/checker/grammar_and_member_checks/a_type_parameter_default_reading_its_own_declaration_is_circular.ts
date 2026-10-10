// @strict: true
// A default that reads its own declaration without supplying the parameter
// needs the default being resolved: TS2716, wherever the reference is read
// eagerly. An object literal's members and a signature are read later.
interface SR<T = SR> {}
interface SR2<T = Array<SR2>> {}
interface SR3<T = { x: SR3 }> {}
interface SR4<T, U = SR4<T>> {}
interface SR5<T, U = SR5<T, T>> {}
class C1<T = C1> {}
type A1<T = A1> = { x: T };
interface SR6<T = SR6 | string> {}
interface SR7<T = () => SR7> {}
interface SR8<T = SR8[]> {}
interface SR9<T = [SR9]> {}
interface SR10<T = keyof SR10> {}
interface SR11<T = SR11["x"]> { x: number }
type A2<T = A2[]> = T;
namespace N { export interface SR<T = SR> {} }
interface SR12<T extends SR12 = SR12> {}
