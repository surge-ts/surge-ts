// @filename: a.ts
export default function () {
    return true;
}
export type X = typeof default;
// @filename: b.ts
export default { a: true }
export type Y = typeof default;
