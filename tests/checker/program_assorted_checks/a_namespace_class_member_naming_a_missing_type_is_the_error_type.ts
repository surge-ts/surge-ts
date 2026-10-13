// @strict: true
// A class declared in a program namespace whose member names a missing type
// returns tsc's error type: a callback passed to a call through it has no
// contextual signature, so its parameter is implicitly `any`.
namespace N {
    export class A {
        m(): Missing {
            return null as any;
        }
        x() {
            const c = this.m();
            c.forEach((item) => {});
        }
    }
    class B {
        m(): Missing {
            return null as any;
        }
        x() {
            this.m().forEach((item) => {});
        }
    }
}
