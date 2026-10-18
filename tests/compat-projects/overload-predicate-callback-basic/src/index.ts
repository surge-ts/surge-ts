export type output<T extends Schema<any, any>> = T["_output"];

export abstract class Schema<Output = any, Input = Output> {
  readonly _output!: Output;
  readonly _input!: Input;
  refine<Refined extends Output>(check: (arg: Output) => arg is Refined): Effects<this, Refined, Input>;
  refine(check: (arg: Output) => unknown): Effects<this, Output, Input>;
  refine(check: (arg: Output) => unknown): Effects<this, any, Input> {
    void check;
    return null as any;
  }
  array(): List<this> {
    return null as any;
  }
}

export class Text extends Schema<string, string> {}
export class List<T extends Schema<any, any>> extends Schema<output<T>[], T["_input"][]> {
  readonly element!: T;
}
export class Effects<T extends Schema<any, any>, Output = output<T>, Input = T["_input"]> extends Schema<Output, Input> {}

declare const text: Text;

// A callback that is no predicate passes over the predicate overload.
const plain = text.refine((value) => value.length > 5);
const plainCheck: Effects<Text, number, string> = plain;

const block = text.refine((value) => {
  return value.length > 1;
});
const blockCheck: number = block;

// A written predicate infers the overload's own type parameter.
const writtenCheck: number = text.refine((value): value is "x" => value === "x");

// The wrappers keep their real arguments down the chain.
const listed = text.refine((value) => value.length > 5).array();
const listedOutput: output<typeof listed> = 1;
const listedOk: output<typeof listed> = ["a"];
void [plainCheck, blockCheck, writtenCheck, listedOutput, listedOk];
