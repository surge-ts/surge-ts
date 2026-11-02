// @surge-compare: messages
// @jsx: preserve
// @strict: true
// @filename: example.tsx
declare global { namespace JSX { interface Element {} interface IntrinsicElements {} } }
type Both<A, B> = A & B;
type Wide = { firstLongPropertyName: string; secondLongPropertyName: number; thirdLongPropertyName: boolean; fourthLongPropertyName: string; fifthLongPropertyName: number; sixthLongPropertyName: string };
declare function Item(props: Both<{ firstLongPropertyName: string; secondLongPropertyName: number; thirdLongPropertyName: boolean; fourthLongPropertyName: string; fifthLongPropertyName: number; sixthLongPropertyName: string }, { label: string }>): JSX.Element;
declare const wide: Wide;
export const excess = <Item {...wide} label="x" extra="y" />;
export const missing = <Item label="x" />;
export const label = <Item {...wide} />;
