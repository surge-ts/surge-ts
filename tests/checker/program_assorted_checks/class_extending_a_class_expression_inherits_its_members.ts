class A extends class B {
	toString() {
		return "B";
	}
	extra = 1;
} {}

const inherited: { extra: number; toString(): string } = new A();
declare const a: A;
const value: number = a.extra;
const missing = a.absent;
const wrong: string = a.extra;

export {};
