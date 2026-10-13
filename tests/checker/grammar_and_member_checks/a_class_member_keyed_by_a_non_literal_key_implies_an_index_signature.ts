declare var something: string;
export const dataSomething = `data-${something}` as const;
export class WithData {
    [dataSomething]?() {
        return "something";
    }
}
export const a = (new WithData())["ahahahaahah"]!();
const staticField = Math.random() > 0.5 ? "s1" : "s2";
export class Holder {
    static [staticField]() {
        return { static: true };
    }
}
export const staticLookup = Holder["some" + "thing"];
