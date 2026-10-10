// @strict: true
// A namespace interface's base is named from the namespace: `Element` here is
// its sibling, not the library's DOM `Element`.
namespace UI {
    export interface Element {}
    export interface ElementClass extends Element {}
    export interface Labeled extends ElementClass { label: string }
}
class Widget implements UI.ElementClass {}
class Button implements UI.Labeled {}
export {};
