// @jsx: preserve
// `JSX.ElementAttributesProperty` names the instance member a class
// component's props are read from; with two members it names none (TS2608 at
// the interface) and the props are the constructor's first parameter.
declare namespace JSX {
	interface Element { }
	interface ElementAttributesProperty { pr1: any; pr2: any; }
	interface IntrinsicElements { }
}

interface Obj1type {
	new(n: string): {};
}
declare var Obj1: Obj1type;
<Obj1 x={10} />;
