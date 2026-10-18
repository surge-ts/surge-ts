// @surge-compare: messages
type Route = "/" | "/docs" | "/play";
type Fn = () => void;
type Both = { a: 1 } & { b: 2 };

declare const aliasUnion: Route[];
aliasUnion.nope;
declare const readonlyAliasUnion: readonly Route[];
readonlyAliasUnion.push("/");
declare const nestedReadonly: readonly Route[][];
nestedReadonly.push([]);
declare const readonlyArrayOf: ReadonlyArray<Route>;
readonlyArrayOf.push("/");
declare const aliasFunction: readonly Fn[];
aliasFunction.push(() => {});
declare const aliasIntersection: readonly Both[];
aliasIntersection.push({ a: 1, b: 2 });

declare const writtenUnion: readonly ("a" | "b")[];
writtenUnion.push("a");
declare const writtenFunction: (() => void)[];
writtenFunction.nope;
declare const writtenIntersection: readonly ({ a: 1 } & { b: 2 })[];
writtenIntersection.push({ a: 1, b: 2 });

const routes: readonly Route[] = ["/"];
const count: number = routes;
routes[0] = "/docs";
