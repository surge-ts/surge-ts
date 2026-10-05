function mk() {
    return class {
        static other() { return 1; }
        m() { return 1; }
    };
}
const M = mk();
const y: string = M.other();
const y2: string = new M().m();
const N = (class { static other() { return 1; } });
const y3: string = N.other();
let L = class Q { static other() { return 1; } };
const y4: string = L.other();
const C9 = class D {
    static other() { return 1; }
    m() { return 1; }
};
const y7: string = C9.other();
const y8: string = new C9().m();
