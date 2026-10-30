// A `declare namespace` member written as a namespace in one declaration file
// and as a class or function in a later one is one merged value, as it is when
// the class or function comes first.
// @surge-compare: messages
// @filename: a_namespaces.d.ts
declare namespace Merged {
	export namespace make {
		export var defaults: { size: number };
	}
	export namespace Shape {
		export var unit: number;
	}
}
// @filename: b_values.d.ts
declare namespace Merged {
	export function make(size: number): string;
	export function make(): string;
	export class Shape {
		constructor(n: number);
		area: number;
	}
}
// @filename: index.ts
const defaults: number = Merged.make.defaults.size;
const made: string = Merged.make(1);
const unit: number = Merged.Shape.unit;
const area: number = new Merged.Shape(1).area;
const missing: number = Merged.make.nope;
