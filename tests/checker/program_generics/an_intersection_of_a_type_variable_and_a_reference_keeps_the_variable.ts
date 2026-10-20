// @surge-compare: messages
interface Box { value: number }
interface Named { name: string }
class Counter { count = 0; inc(): void { this.count++; } }

function keepsVariable<T>(x: T & Box): T { return x; }
function readsMember<T>(x: T & Box): number { return x.value; }
function toReference<T>(x: T & Box): Box { return x; }
function readsConstraint<T extends Named>(x: T & Box): string { return x.name; }
function twoReferences<T>(x: T & Box & Named): T { return x; }
function aClassInstance<T>(x: T & Counter): T { x.inc(); return x; }
function keys<T>(x: T & Box): keyof (T & Box) { return "value"; }

function notAssignable<T>(x: T): T & Box { return x; }
function missingMember<T>(x: T & Box): string { return x.missing; }
function notFromOtherReferences<T>(y: Box & Named): T & Box { return y; }

type Boxed<T> = T & Box;
type WithPath<T> = Boxed<T> & { path: string };
function nested<T>(o: WithPath<T>): number { return o.value + o.path.length; }
function nestedKeepsVariable<T>(o: WithPath<T>): T { return o; }

declare function makeResource<T>(thing: T): T & Box;
const resource = makeResource(() => 1);
const total: number = resource() + resource.value;
