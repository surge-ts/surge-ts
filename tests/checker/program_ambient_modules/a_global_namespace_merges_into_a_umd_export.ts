// @filename: a.d.ts
export as namespace a;
export const x = 0;
// @filename: b.ts
import * as a2 from "./a";
declare global {
    namespace a {
        export const y = 0;
    }
}
declare module "./a" {
    export const z = 0;
}
a.x + a.y + a.z;
a2.x + a2.y + a2.z;
a.missing;
a2.missing;
