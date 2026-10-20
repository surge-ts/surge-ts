// @strict: true
// @exactOptionalPropertyTypes: true
// `"p" in o` and `o.hasOwnProperty("p")` prove an exact optional `p` present
// in a conditional expression too, so its read loses the missing `undefined`.
interface Options {
    label?: string;
}
export function ternary(options: Options) {
    const t1: string = "label" in options ? options.label : "x";
    const t2: string = options.hasOwnProperty("label") ? options.label : "x";
    const t3: number = "label" in options ? options.label : 1;
    return [t1, t2, t3];
}
