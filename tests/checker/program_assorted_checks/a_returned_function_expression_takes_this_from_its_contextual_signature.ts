// @strict: true
function untyped(): () => number {
    return function () { return this.q; };
}
function typed(): (this: { q: number }) => number {
    return function () { return this.q; };
}
function inferred() {
    return function () { return this.r; };
}
const literal = {
    m(): () => void {
        return function () { this.s; };
    },
};
