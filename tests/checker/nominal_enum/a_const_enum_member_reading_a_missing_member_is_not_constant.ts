// A const enum initializer reading a member that does not exist has no
// constant value (TS2474), besides the missing member (TS2339).
const enum E1 {
    A = 1,
    Y = E1.Z,
    Y1 = E1["Z"],
    B = E1.A,
}
