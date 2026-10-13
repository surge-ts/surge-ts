// @filename: a.tsx
// @jsx: preserve
// @strict: true
declare namespace JSX {
    interface Element {}
    interface ElementClass extends Element {}
    interface ElementAttributesProperty { props: {}; }
    interface IntrinsicElements { [key: string]: Element }
}
declare class Component<P> {
    constructor(props: Readonly<P>);
    readonly props: Readonly<P>;
}
class C<T> extends Component<{ x?: boolean; } & T> {}
const x = <C foobar="example" />;
class ElemClass<T extends { x: number }> implements JSX.ElementClass {
    constructor(public props: T) {}
}
const elem = <ElemClass x={12} y={24} />;
