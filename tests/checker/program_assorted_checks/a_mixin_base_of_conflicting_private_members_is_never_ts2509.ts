class Private {
    constructor(...args: any[]) {}
    private p!: string;
}
class Private2 {
    constructor(...args: any[]) {}
    private p!: string;
}
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

declare function Mix<T, U>(c1: T, c2: U): T & U;

class C1 extends Mix(Private, Private2) {}
class C2 extends Mix(Private, Protected) {}
class C3 extends Mix(Private, Public) {}
class C4 extends Mix(Protected, Protected2) {}
class C5 extends Mix(Protected, Public) {}
class C6 extends Mix(Public, Public) {}
