// A class expression's own name is bound in its body, where its members'
// inferred returns read it.
const C = class D {
    static #field = 1;
    static getClass() { return D; }
    getSelf() { return new D(); }
};
const n1: number = C.getClass();
const n2: number = new C().getSelf();
C.getClass().#field;
