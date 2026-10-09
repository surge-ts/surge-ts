// @strict: true
// @lib: es2023
namespace Geo {
  export interface Point { x: number; y: number }
  export function dist(a: Point, b: Point) { return Math.hypot(a.x - b.x, a.y - b.y); }
  export function origin() { return { x: 0, y: 0 }; }
  export function written(): number { return 1; }
}
const distance: string = Geo.dist({ x: 0, y: 0 }, { x: 1, y: 1 });
const origin: number = Geo.origin();
const written: string = Geo.written();
const shadowed = "module";
namespace Local {
  const shadowed = 1;
  export function read() { return shadowed; }
}
const readBack: number = Local.read();
