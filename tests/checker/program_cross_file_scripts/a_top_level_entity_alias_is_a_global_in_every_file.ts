// A script's top-level `import X = N.M` names N.M's type and value for every
// file, and leaves the file a script: in a declaration file as in a source.
// @filename: types.d.ts
declare namespace AliasedTypes {
	export interface AliasSet<T> {
		size(): number;
		has(value: T): boolean;
	}
	export interface AliasBox<T> {
		value: T;
	}
	export const AliasBox: { resolve<T>(value: T): AliasBox<T> };
	export function identity<T>(value: T): T;
}
import AliasSet = AliasedTypes.AliasSet;
import AliasBox = AliasedTypes.AliasBox;
import identity = AliasedTypes.identity;
// @filename: script.ts
namespace Local {
	export const x = 1;
	export interface I {
		v: number;
	}
}
import x2 = Local.x;
import I2 = Local.I;
// @filename: index.ts
declare const values: AliasSet<number>;
const size: string = values.size();
const boxed = AliasBox.resolve(1);
const value: string = boxed.value;
const same: string = identity(123);
const read: string = x2;
declare const i: I2;
const member: string = i.v;
