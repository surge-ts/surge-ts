// @lib: es2023,dom
// An interface's own index signature must stay assignable to its base's,
// however large the DOM types they index.
interface Base {
    [index: number]: HTMLElement;
}
interface Narrower extends Base {
    [index: number]: Element;
}
interface Keyed {
    [index: number]: HTMLElement;
    x: number;
}
interface KeyedNarrower extends Keyed {
    [index: number]: Element;
}
