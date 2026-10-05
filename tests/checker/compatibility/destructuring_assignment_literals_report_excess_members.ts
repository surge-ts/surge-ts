// @strict: true
function f4() {
    var x: number, y: number;
    ({ } = { x: 0, y: 0 });
    ({ x } = { x: 0, y: 0 });
    ({ y } = { x: 0, y: 0 });
    ({ x, y } = { x: 0, y: 0 });
    ({ x } = { x: 0, y: 0, z: 1 });
    let o: any;
    ({ x, ...o } = { x: 0, y: 0 });
    let s: string;
    ({ x: s } = { x: "", q: 1 });
    ({ x = 1 } = { y: 0 });
}
