// A class expression a body class's static member holds reads that body's
// bindings, the class itself included: its method returns `A`, not `any`.
export function outer() {
	class A {
		static member = class B {
			method() {
				return new A();
			}
		};
	}
	const read: string = new A.member().method();
}
