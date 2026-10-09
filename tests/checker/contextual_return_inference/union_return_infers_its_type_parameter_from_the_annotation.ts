// @strict: true
// @lib: es2022,dom
interface Base { id: string }
interface Rect extends Base { width: number }
declare const doc: { querySelector<E extends Base = Base>(selectors: string): E | null };
const viaMember: Rect | null = doc.querySelector(".x");
declare function find<E extends Base = Base>(selectors: string): E | undefined;
const viaFunction: Rect | undefined = find(".x");
const element: SVGRectElement | null = document.querySelector(".svg-rectangle");
const asserted: SVGRectElement = document.querySelector(".svg-rectangle")!;
const mismatched: number | null = doc.querySelector(".x");
