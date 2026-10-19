// @types: kinds
// A namespace function a dependency declares returns the namespace's own
// types although one of them reads the namespace back (`Owner: typeof
// Kind.Axis`, as roblox-ts's enum items do), not the permissive stand-in.
// @filename: node_modules/@types/kinds/package.json
{ "name": "@types/kinds", "version": "1.0.0", "types": "index.d.ts" }
// @filename: node_modules/@types/kinds/index.d.ts
interface Item {
	Name: string;
}
declare namespace Kind {
	export namespace Axis {
		export interface X extends Item {
			Owner: typeof Kind.Axis;
		}
		export const X: X;
		export function GetItems(): Array<Kind.Axis>;
		export function FromName(name: string): Kind.Axis | undefined;
	}
	export type Axis = Axis.X;
}
// @filename: index.ts
const items: string = Kind.Axis.GetItems();
const named: string = Kind.Axis.FromName("X");
export {};
