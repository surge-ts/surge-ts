// A class member's return read from its body sees `super` as the member's
// check does: the base class's instance in an instance member and its
// constructor in a static one, in a body-local class as at the top level.
class Base {
	static method(value: number) { return value; }
	read() { return 1; }
}
class Derived extends Base {
	static check() { return super.method(40); }
	read() { return super.read() + 1; }
	label() { return super.read().toFixed(); }
}
const viaStatic: string = Derived.check();
const viaInstance: string = new Derived().read();
const viaChain: number = new Derived().label();

export function local() {
	class LocalBase {
		static value = 1;
		static method(value: number) { return this.value + value; }
		read() { return "base"; }
	}
	class LocalDerived extends LocalBase {
		static check() { return super.method(40); }
		size() { return super.read().length; }
	}
	const localStatic: string = LocalDerived.check();
	const localInstance: string = new LocalDerived().size();
}
