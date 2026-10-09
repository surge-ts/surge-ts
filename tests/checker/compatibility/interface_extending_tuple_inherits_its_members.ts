// @strict: true
type EmptyTuple = [];
interface MyEmptyTuple extends EmptyTuple { extraInfo?: any; }
declare const empty: MyEmptyTuple;
export const length: 1 = empty.length;
export const map: 0 = empty.map;
const withExtra: MyEmptyTuple = [];
type Pair = [number, string];
interface Labelled extends Pair { label?: string }
export const pair: Labelled = [1, 'a'];
declare const labelled: Labelled;
export const first: 0 = labelled[0];
export const size: 0 = labelled.length;
export const joined: number = labelled.join();
