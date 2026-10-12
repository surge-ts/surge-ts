// @surge-compare: messages
export namespace ns {
    export function make<N extends string>(name: N): Placeholder<N> {
        return new Placeholder(name);
    }
    export function literal() {
        const p = new Placeholder("lit");
        const n: number = p;
        const q: Placeholder<"lit"> = new Placeholder("lit");
    }
}
export class Placeholder<TName extends string = string, TValue = any> {
    declare protected: TValue;
    constructor(readonly name: TName) {}
}
