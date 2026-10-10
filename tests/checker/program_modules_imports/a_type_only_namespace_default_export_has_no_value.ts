// @allowJs: true
// @checkJs: true
// @strict: true
// A default export naming a namespace of types alone exports no value: a
// JavaScript file cannot import it (TS18042), and a value use of the import is
// TS2708. A namespace with a value member exports that value.
// @filename: types.d.ts
declare module "contract" {
    interface ContractObject { foo: number; }
    namespace TruffleContract {
        export type Contract = ContractObject;
    }
    export default TruffleContract;
}
declare module "valued" {
    namespace Valued {
        export const version: number;
        export type Shape = { a: string };
    }
    export default Valued;
}
// @filename: caller.js
import TruffleContract from 'contract';
console.log(typeof TruffleContract, TruffleContract);
// @filename: main.ts
import TC from 'contract';
import Valued from 'valued';
console.log(TC);
const c: TC.Contract = { foo: 1 };
const v: number = Valued.version;
const s: Valued.Shape = { a: "" };
