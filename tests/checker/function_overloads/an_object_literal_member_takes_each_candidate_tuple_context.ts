// @strict: true
// Each overload candidate types an object literal argument's array members in
// that candidate's tuple context, as `spawn(cmd, args, { stdio: [...] })` picks
// its `[StdioNull, StdioPipe, StdioPipe]` overload.
type Null = "inherit" | "ignore";
type Pipe = undefined | null | "pipe" | "overlapped";
interface Base { cwd?: string; stdio?: string | Array<string | null | undefined> }
interface Tuple<A extends Null | Pipe, B extends Null | Pipe> extends Base { stdio: [A, B] }
declare function spawn(c: string, o: Tuple<Pipe, Pipe>): 1;
declare function spawn(c: string, o: Tuple<Null, Pipe>): 2;
declare function spawn(c: string, o: Base): 3;
export const alone: 2 = spawn("a", { stdio: ["ignore", "pipe"] });
export const withCwd: 2 = spawn("a", { cwd: "x", stdio: ["ignore", "pipe"] });
export const piped: 1 = spawn("a", { stdio: ["pipe", "pipe"] });
