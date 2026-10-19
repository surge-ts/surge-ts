// @module: commonjs
// @allowSyntheticDefaultImports: true
// A namespace's `export { X }` of a value its file imports is that value
// (`@rbxts/roact`'s `Roact.Children`: the default import of an `export =`
// unique symbol), read off the namespace, as a computed key, and by name.
// @filename: node_modules/rx/package.json
{ "name": "rx", "version": "1.0.0", "types": "index.d.ts" }
// @filename: node_modules/rx/Children.d.ts
import Rx from "./index";
declare const Children: unique symbol;
type Children = Map<string | number, Rx.Element>;
export = Children;
// @filename: node_modules/rx/index.d.ts
import Children from "./Children";
declare namespace Rx {
	export { Children };
}
declare namespace Rx {
	export interface Element {
		props: object;
	}
}
export = Rx;
// @filename: index.ts
import Rx from "rx";
import { Children } from "rx";
const key: string = Rx.Children;
const named: string = Children;
type Explicit = { [Rx.Children]: { [name: string]: Rx.Element | undefined } };
declare const props: Explicit;
const child: string = props[Rx.Children].Testing;
export {};
