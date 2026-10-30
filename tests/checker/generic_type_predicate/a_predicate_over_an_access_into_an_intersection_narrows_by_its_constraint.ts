// @strict: true
// `TagMap[T]` over an intersection is constrained by the members its operands
// merge to, so a predicate asserting it narrows `Base | null` to it, and an
// access read as a `string` is checked against that constraint.
interface Base {
	kind: string;
}
interface Div extends Base {
	div: true;
}
interface Span extends Base {
	span: true;
}
interface Svg extends Base {
	svg: true;
}
interface Html {
	div: Div;
	span: Span;
}
interface Graphics {
	svg: Svg;
	span: Svg;
}
type TagMap = Html & Pick<Graphics, Exclude<keyof Graphics, keyof Html>>;

declare function isTag<T extends keyof TagMap, U extends TagMap[T]>(node: Base | null, tag: T): node is U;

export function narrowed<T extends keyof TagMap>(node: Base | null, tag: T) {
	if (isTag(node, tag)) {
		node.kind;
	}
}

export function read<T extends keyof (Html & Graphics)>(value: (Html & Graphics)[T]) {
	const text: string = value;
	return text;
}
