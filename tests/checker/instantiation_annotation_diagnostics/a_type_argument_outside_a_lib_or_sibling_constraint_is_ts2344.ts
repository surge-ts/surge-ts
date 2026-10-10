// @strict: true
// A primitive lacks members a lib interface constraint requires, however
// deep that interface's own members go; an unconstrained type variable is
// `unknown`, which a constraint naming its siblings does not admit either.
function g1<T extends EventTarget>(producer: () => T) { }
g1<number>(() => 3);
interface Lifecycle<Attrs, State> {
    oninit?(vnode: Vnode<Attrs, State>): number;
}
interface Vnode<Attrs, State extends Lifecycle<Attrs, State>> {
    attrs: Attrs;
}
