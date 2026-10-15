// @jsx: preserve
// @strict: true
import { Missing, Props } from "missing";
declare namespace JSX { interface Element {} interface IntrinsicElements { [k: string]: any } }
declare function L2<T extends Missing = Missing>(props: Omit<Missing, "as"> & { as?: T }): JSX.Element;
declare function L8<T extends Missing = Missing>(props: Omit<Props<T>, "as"> & { as?: T }): JSX.Element;
declare function L12<T extends Missing = Missing>(props: Pick<Props<T>, "x"> & { as?: T }): JSX.Element;
<L2 onClick={(e) => {}} />;
<L8 onClick={(e) => {}} />;
<L12 x={(e) => {}} />;
const picked: number = null! as Pick<Missing, "x">;
