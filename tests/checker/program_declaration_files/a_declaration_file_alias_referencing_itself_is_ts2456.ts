// A project's own declaration file is checked for a type alias that
// references itself (TS2456).
// @filename: types.d.ts
type A = A;
type B = C;
type C = B;
