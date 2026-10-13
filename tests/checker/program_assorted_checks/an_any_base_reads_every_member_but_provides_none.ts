// tsc's `anyBaseTypeIndexInfo`: a class extending an `any` value reads any
// member through a string index of `any`, but that index gives a relation no
// property.
declare var Base: any;
export class Empty extends Base.Component<{}, {}> {
    render() { return 1; }
}
export class Plain extends Base {
    render() { return 1; }
}
declare const e: Empty;
declare const p: Plain;
const read: number = e.anything;
const related: { props: number } = e;
const plainRelated: { state: string } = p;
const plainRead: string = p.whatever;
