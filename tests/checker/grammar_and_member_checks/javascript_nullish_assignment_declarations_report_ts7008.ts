// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @filename: a.js
class C {
    constructor() {
        this.inConstructor = undefined;
        this.typed = null;
        this.later = null;
    }
    method() {
        if (Math.random()) {
            this.inMethodNullable = null;
        } else {
            this.inMethodNullable = undefined;
        }
        this.later = 1;
        const arrow = () => { this.inArrow = void 0; };
        function inner() { this.notMine = 1; }
    }
    static s() {
        this.staticNullable = null;
    }
}

class Declared {
    field = null;
    m() { this.field = null; }
}

var o = {};
o.x = 1;
o.y = void 0;
o.x + o.y;

function F() {}
F.nullable = null;
F.number = 0;

const notExpando = { a: 1 };
