// An element is typed by its own file's `@jsx` factory namespace wherever it
// is read, and several children relate one by one to an array `children` type.
// @jsx: react
// @filename: renderer.d.ts
export namespace dom {
    namespace JSX {
        interface IntrinsicElements { [e: string]: {}; }
        interface Element { __domBrand: void; props: { children?: Element[] }; }
        interface ElementAttributesProperty { props: any; }
        interface ElementChildrenAttribute { children: any; }
    }
}
export function dom(): dom.JSX.Element;
// @filename: renderer2.d.ts
export namespace predom {
    namespace JSX {
        interface IntrinsicElements { [e: string]: {}; }
        interface Element { __predomBrand: void; props: { children?: Element[] }; }
        interface ElementAttributesProperty { props: any; }
        interface ElementChildrenAttribute { children: any; }
    }
}
export function predom(): predom.JSX.Element;
// @filename: component.tsx
/** @jsx predom */
import { predom } from "./renderer2"
export const tree = <h></h>;
export default <h></h>
// @filename: index.tsx
/** @jsx dom */
import { dom } from "./renderer"
import prerendered, { tree } from "./component";
let elem = prerendered;
elem = <h></h>;
const DOMSFC = (props: { x: number, children?: dom.JSX.Element[] }) => <p></p>;
const brokenTree = <DOMSFC x={1}>{tree}{tree}</DOMSFC>;
const fine = <DOMSFC x={1}><p></p><p></p></DOMSFC>;
