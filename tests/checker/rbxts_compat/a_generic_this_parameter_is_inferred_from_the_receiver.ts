// A signature whose written `this` is its own type parameter infers it from
// the call's receiver (`inferTypeArguments` reads the `this` argument), so a
// method returning `this` returns the object it is called on.
interface Numerable {
	n: number;
}
function add(this: Numerable, n: number) {
	return (this.n += n);
}
const mul = function <T extends Numerable>(this: T, n: number) {
	this.n *= n;
	return this;
};
const obj = { add, n: 10, mul };
const same: string = obj.mul(3);
const chained: string = obj.mul(3).add(6);
