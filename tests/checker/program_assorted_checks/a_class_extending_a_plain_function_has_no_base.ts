// A class whose base is a plain function — no construct signature, TS2507 —
// has no base type at all (`resolveBaseTypesOfClass`): no inherited members,
// and the default constructor takes nothing.
function Dragon(numberEaten: number) {}
class Firedrake extends Dragon {
    x = 1;
}
const fd = new Firedrake();
fd.x;
fd.zzz;
new Firedrake(1);
