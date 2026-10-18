// A class declared in a function body types its unannotated members with
// that body's bindings in scope, as tsc's closure sees them: a static member's
// `this` is the class, the class reads itself by name, and a member reads the
// enclosing locals, including one declared after the class. A top-level
// class's member still reads the module's binding, not a caller's local.
export function outer() {
	const local = 1;
	class X {
		value = 1;
		static label = "s";
		static viaThis() { return this.label; }
		static viaName() { return X.label; }
		static readsLocal() { return local; }
		static readsLater() { return later; }
		static async chained() { return (await this.viaPromise()).length; }
		static async viaPromise() { return "foo"; }
		instanceReadsLocal() { return local; }
		instanceReadsThis() { return this.value; }
	}
	const later = "later";
	const a: boolean = X.viaThis();
	const b: boolean = X.viaName();
	const c: boolean = X.readsLocal();
	const d: boolean = X.readsLater();
	const e: boolean = X.chained();
	const f: boolean = new X().instanceReadsLocal();
	const g: boolean = new X().instanceReadsThis();
}

export function callback(run: (f: () => void) => void) {
	run(() => {
		class Y {
			static async foo() { return "foo"; }
			static async bar() { return (await this.foo()) + "bar"; }
		}
		const p: number = Y.bar();
	});
}

class Top {
	read() { return shadowed; }
}
const shadowed = 1;
export function user() {
	const shadowed = "s";
	const t: boolean = new Top().read();
}
