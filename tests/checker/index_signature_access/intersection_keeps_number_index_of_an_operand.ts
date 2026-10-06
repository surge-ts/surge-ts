// @noImplicitAny: true
// An intersection keeps the index signatures of its operands, so a string key
// on one whose operand has only a number index is TS7015 on the key.
interface Numbered { [index: number]: string; b: 1 }
declare let numbered: Numbered & { a: 1 };
numbered["hi"];
numbered[0].length;
declare let win: Window & typeof globalThis;
win["hi"];
const first: Window = win[0];
const actions = ["resizeTo", "resizeBy"] as const;
for (const action of actions) {
    win[action] = (x, y) => {
        win[action](x, y);
    };
}
