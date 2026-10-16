// @jsx: react
// @strict: true
declare global { namespace JSX { interface Element {} interface IntrinsicElements { p: any } } }
declare var React: any;
declare const u: undefined;
declare const items: string[];
export const A = () => <p>{...this.props.children}</p>;
export const B = () => <p>{...u.props}</p>;
export const C = () => <p>{...items}</p>;
