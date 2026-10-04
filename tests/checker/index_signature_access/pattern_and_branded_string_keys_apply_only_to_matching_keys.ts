// @noImplicitAny: true
type Tag1 = { __tag1__: void };
type Tag2 = { __tag2__: void };
type TaggedString1 = string & Tag1;
type TaggedString2 = string & Tag2;

declare let s0: string;
declare let s1: TaggedString1;
declare let s2: TaggedString2;
declare let s3: TaggedString1 | TaggedString2;

interface I1 { [key: TaggedString1]: string }
interface I2 { [key: TaggedString2]: string }
interface I3 { [key: TaggedString1 | TaggedString2]: string }

declare let i1: I1;
declare let i2: I2;
declare let i3: I3;

i1[s0];
i1[s1];
i1[s2];
i1[s3];
i3[s3];

i1 = i2;
i1 = i3;
i3 = i1;

declare let dom: { [x: `data${string}`]: string };
const y1 = dom["data123"];
const y2 = dom["date123"];
const y3 = dom.data123;
const y4 = dom.date123;
dom = { data123: "hello" };
dom = { date123: "hello" };

type Funcs = {
    [key: `s${string}`]: (x: string) => void,
    [key: `n${string}`]: (x: number) => void,
};
const funcs: Funcs = {
    sfoo: x => x.length,
    nfoo: x => x * 2,
};

interface DataAttributes {
    [key: `data-${string}`]: string;
    id: number;
}
