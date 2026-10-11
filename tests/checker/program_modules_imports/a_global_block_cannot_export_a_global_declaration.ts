// @filename: module.ts
const z = 1;
export {};
declare global {
    var y: number;
    interface Local {}
    export { globalThis as global }
    export { y as yy }
    export { Array as Arr }
    export { z as zz }
    export { undefined as u }
    export { string as s }
    export { Local as L }
    export type { Date as D }
}
// @filename: script.ts
var w: number;
interface Q {}
declare global {
    var v: number;
    export { globalThis as global2 }
    export { w as ww }
    export { Q as QQ }
    export { v as vv }
}
