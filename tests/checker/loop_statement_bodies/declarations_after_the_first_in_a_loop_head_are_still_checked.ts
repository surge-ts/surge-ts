for (const a, { [b]: c } of [1]) {
}
for (let x, { [d]: e } in {}) {
}
function inside() {
    for (const a2, { [b2]: c2 } of [1]) {}
    for (var y, z of [1]) {}
}
