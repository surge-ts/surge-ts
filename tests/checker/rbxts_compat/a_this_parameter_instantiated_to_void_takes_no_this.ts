// `checkApplicableSignature` skips the `this` check for a `this` type that
// is `void` once instantiated: `this: Exclude<T, undefined>` under `T = void`.
export {};
function make<T>() {
	return {
		method(this: Exclude<T, undefined>, value: number) {
			return value + 1;
		},
	};
}
make<void>().method(41);
make<string>().method(41);
type Self<T> = T extends unknown ? T : never;
declare const receiver: { method(this: Self<void>, value: number): number };
receiver.method(1);
