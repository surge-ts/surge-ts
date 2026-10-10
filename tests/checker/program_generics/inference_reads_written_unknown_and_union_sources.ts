// An argument built only from declared values carries the `unknown` its
// declarations wrote, and a union source is inferred from member by member.
declare function g<T>(a: { x: T }[], b: T): T;
declare const u: { x: unknown };
const q = g([u], null);
const s: string = q;

type Op<TInput> = { id: number; input: TInput };
type Link<TR, TInput = unknown, TOutput = unknown> = (opts: {
    op: Op<TInput>;
    next: (op: Op<TInput>) => TOutput;
}) => TR;
declare function createChain<TR, TInput = unknown, TOutput = unknown>(opts: {
    links: Link<TR, TInput, TOutput>[];
    op: Op<TInput>;
}): TInput;
declare const declared: Link<string>;
declare function makeLink<TR = string>(): () => Link<TR>;
const made = makeLink()();
const input = createChain({ links: [made, declared], op: { id: 1, input: null } });
const t: string = input;
