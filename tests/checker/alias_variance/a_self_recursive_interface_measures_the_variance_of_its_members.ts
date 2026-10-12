interface Box<in out T> { value: T }
interface Holder<T> { take: (box: Box<T>) => void; self: Holder<T>; }
declare const h: Holder<"a">;
const widened: Holder<"a" | "b"> = h;
interface Listener<T> { listen: (event: T) => void; next: Listener<T> }
declare const l: Listener<"a">;
const wider: Listener<"a" | "b"> = l;
const narrower: Listener<"a"> = null! as Listener<"a" | "b">;
