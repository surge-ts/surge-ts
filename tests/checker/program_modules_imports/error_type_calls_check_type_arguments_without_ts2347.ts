// @filename: call.ts
import { foo } from "unknown";
type T = number;
foo<T>();
foo<Missing>();
// @filename: super.ts
import { A, B } from "unknown";
type T = number;
export class C extends A<B> {
    m() {
        super.m<T>(1);
        super.m<InvalidReference>();
    }
}
