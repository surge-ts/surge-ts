// @strict: true
// A class merged with an interface shares its type parameters, constrained by
// whichever declaration writes a constraint; an unmerged class's own
// unconstrained parameter is still `unknown` against the same constraint.
type Selection = Record<string, unknown>;
type Build<T extends Selection> = { [K in keyof T]: T[K] };
export interface Merged<TSel extends Selection, TOut = Build<TSel>> {
    out: TOut;
}
export class Merged<TSel, TOut = Build<TSel>> {
    read(value: Build<TSel>): TSel {
        return value as TSel;
    }
    keys(): (keyof TSel)[] {
        return [];
    }
}
export class Lone<TSel, TOut = Build<TSel>> {
    out!: TOut;
}
