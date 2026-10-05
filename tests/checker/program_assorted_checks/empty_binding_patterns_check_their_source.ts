// @strict: true
const [] = {};
const {} = undefined;
(([]) => 0)({});
function foo({}: undefined) {
    return 0;
}
function bar([]: {}) {
    return 0;
}
const { }: undefined = 1;
const []: {} = {};
declare const maybe: string | null;
const {} = maybe;
let [] = [1];
const {} = { a: 1 };
function inner() {
    const [] = 1;
    const {} = null;
}
