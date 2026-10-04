class Protected {
    constructor(...args: any[]) {}
    protected p!: string;
}
class Protected2 {
    constructor(...args: any[]) {}
    protected p!: string;
}
class Public {
    constructor(...args: any[]) {}
    public p!: string;
}
function f4(x: Protected & Protected2) {
    x.p;
}
function f5(x: Protected & Public) {
    x.p;
}
