// @allowJs: true
// @checkJs: true
// @filename: a.js
function Test({ b = '' } = {}) {}
Test(({ b = '5' } = {}));
Test(({ b: e } = { b: 'x' }));
const g = ([h] = [2]);
function inner() {
    Test(({ m = 1 } = {}));
    let n;
    const o = ({ n = 1 } = {});
    return [o, n];
}
let known = 0;
const read = ({ known } = { known: 1 });
