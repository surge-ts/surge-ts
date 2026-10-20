// @strict: false
// An unannotated destructured parameter of a signature takes the type its
// pattern implies (`getTypeFromBindingPattern`): a tuple of its positions, an
// object of its names.
interface Methods {
    tuple([, a, b]): void;
    object({ p, m: { q } }): void;
}
declare const methods: Methods;
methods.tuple([1]);
methods.tuple([1, 2, 3]);
methods.object({ p: 1 });
methods.object({ p: 1, m: { q: 2 } });
