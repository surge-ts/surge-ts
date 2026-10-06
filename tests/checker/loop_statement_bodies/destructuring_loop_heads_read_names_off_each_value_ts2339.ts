declare const ys: { y: number }[];
for (const {x} of ys) {}
for (const {x = 1} of [{y: 1}]) {}
for (const {y} of [{y: 1}]) {}
for (const [p, q] of [[1, 2]]) {}
for (var {a, b} in []) {}
function f(items: { id: number }[]) {
    for (const { id, name } of items) {}
}
