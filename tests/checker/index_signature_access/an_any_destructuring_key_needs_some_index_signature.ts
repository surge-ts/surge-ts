// An `any` key reaches every index signature (`getApplicableIndexInfo`), so a
// destructured property read through one is TS2538 only from a parent that
// declares none.
declare const key: any;
const { [key]: fromObject } = {} as { a: string };
const { [key]: fromNumberIndex } = {} as { [k: number]: string };
const { [key]: fromArray } = [1, 2];
const { [key]: fromString } = "text";
const { [key]: fromNumber } = 5;
let assigned: string;
({ [key]: assigned } = {} as { a: string });
function annotated({ [key]: parameter }: { a: string }) {
    return parameter;
}
export { fromObject, fromNumberIndex, fromArray, fromString, fromNumber, annotated };
