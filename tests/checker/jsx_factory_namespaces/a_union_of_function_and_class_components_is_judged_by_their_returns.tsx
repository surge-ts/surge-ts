// @jsx: preserve
// @strict: true
// A union of a function and a class component has neither kind's signatures
// itself: what its members return or construct must suit `Element | null |
// ElementClass` together, else TS2786 at the tag.
namespace JSX {
  export interface Element {
    type: 'element';
  }
  export interface ElementClass {
    type: 'element-class';
  }
}
function FunctionComponent<T extends string>({type}: {type?: T}) {
  return {
    type
  }
}
class ClassComponent {
  type = 'string';
}
function GoodFunction() {
  return { type: 'element' as const };
}
class GoodClass {
  type = 'element-class' as const;
}
const MixedComponent = Math.random() ? FunctionComponent : ClassComponent;
const GoodMixed = Math.random() ? GoodFunction : GoodClass;
const elem4 = <MixedComponent />;
const elem5 = <GoodMixed />;
