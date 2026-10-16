// @jsx: react
// @strict: true
declare global {
    namespace JSX {
        interface Element {}
        interface ElementAttributesProperty { props: {} }
        interface IntrinsicElements { div: any }
    }
}
declare var React: any;
export class Empty extends React.Component<{}, {}> {
    render() { return <div>Hello</div>; }
}
declare const obj: { a: number | undefined } | undefined;
let spread = <Empty {...obj} />;
let attribute = <Empty a={1} />;
let bare = <Empty />;
