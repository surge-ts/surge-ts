// @surge-compare: messages
type Route = "/" | "/docs" | "/play";
type MaybeRoute = Route | undefined;

declare const routeOrNumber: Route | number;
routeOrNumber.nope;
declare const inArray: Array<Route | number>;
inArray.nope;
declare const inObject: { a: Route | number };
inObject.nope;
declare const readonlyMixed: readonly (Route | number)[];
readonlyMixed.push(1);
declare const nullable: (Route | null)[];
nullable.nope;
declare const viaAlias: MaybeRoute[];
viaAlias.nope;
