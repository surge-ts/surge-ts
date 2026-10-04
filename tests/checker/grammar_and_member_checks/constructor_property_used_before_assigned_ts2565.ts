// @strict: true
// @target: es2022
class C {
    a: number;
    b: string | null;
    #d: number;
    later: number;
    constructor() {
        let x = this.a;
        this.a = this.b;
        this.b = this.#d;
        this.#d = 1;
        const read = () => this.later;
        this.later = 2;
        this.later;
    }
}
