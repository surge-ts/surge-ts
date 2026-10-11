// A nested namespace's member read by its `Namespace.member` spelling from a
// sibling namespace keeps its own heritage: `I2 extends M` resolves where I2
// is declared, through the interface `M` merged with namespace `M`.
declare namespace Outer {
    interface M {
        size: number;
    }
    namespace M {
        interface I2 extends M {
            extra: string;
        }
    }
    namespace Other {
        const t1: M.I2;
        interface Wrap {
            w: M.I2;
        }
    }
    interface Seq extends M {
        size: number | undefined;
        clone(): this;
    }
    namespace Seq {
        interface Keyed extends Seq, M.I2 {}
    }
    namespace M {
        interface Copy extends Seq {
            clone(): Copy;
        }
    }
}
export const fromValue: string = Outer.Other.t1.size;
declare const wrap: Outer.Other.Wrap;
export const fromMember: string = wrap.w.size;
