// @jsx: preserve
// @strict: true
namespace JSX {
  export interface Element { type: 'element'; }
  export interface ElementClass { type: 'element-class'; }
}
function FunctionComponent<T extends string>({ type }: { type?: T }) {
  return { type };
}
FunctionComponent.useThis = function () {
  return <this type="foo" />;
};
class ClassComponent { type = 'string'; }
const MixedComponent = Math.random() ? FunctionComponent : ClassComponent;
const mixed = <MixedComponent />;
