// `import x = N.M` inside a namespace carries every meaning of its entity:
// N.M's type is reachable through the alias, and an alias with a type meaning
// cannot take a predefined type's name (TS2438).
namespace source {
    export type Count = number;
    export class Box {
        value = 1;
    }
    export namespace Inner {
        export interface Shape {
            size: number;
        }
    }
}
namespace aliases {
    export import Count = source.Count;
    export import Box = source.Box;
    export import Shape = source.Inner.Shape;
    import Local = source.Count;
    export const local: Local = 1;
}
let count: aliases.Count = "text";
let box: aliases.Box = { value: "text" };
let shape: aliases.Shape = { size: "big" };
namespace reserved {
    import any = source.Count;
    import string = source.Inner;
}
export {};
