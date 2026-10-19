// @noImplicitAny: true
// `checkObjectLiteral`: a computed name typed `number` adds no member but a
// number index signature whose value is that member's type, for a method as
// for a property, beside members a literal value widens.
export {};
let i = 0;
const property = { calls: 0, [++i]: 1 };
const fromProperty: string = property[i];
const method = { calls: 0, [++i](n: 5) { return 1; } };
const fromMethod: string = method[i];
const named = { [i](n: 5) { return 1; } };
const fromNamed: string = named[i];
const withThis = {
	calls: 0,
	[++i](n: 5) {
		return this.f(n);
	},
	f(n: 5) {
		return ++this.calls;
	},
};
const fromThis: string = withThis[i];
