// However much of a large lib graph surge leaves unwalked, a primitive has no
// call signature and a function returning `void` returns no object, so writes
// into DOM members are still related.
// @lib: dom,esnext
declare let factory: () => EventTarget;
declare const count: number;
factory = count;
declare const element: Element;
element.animate = function () {};
element.animate = 5;
element.getAttribute = function (name: string) { return name; };
