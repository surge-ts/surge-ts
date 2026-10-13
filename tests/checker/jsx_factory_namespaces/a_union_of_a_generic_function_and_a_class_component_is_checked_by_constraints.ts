// @filename: a.tsx
// @strict: true
// @jsx: preserve
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

const MixedComponent = Math.random() ? FunctionComponent : ClassComponent;
const elem4 = <MixedComponent />;
