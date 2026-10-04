class MyClass {
    private p: number = 1;
    protected pp: number = 1;
    public ppp: number = 1;
    extension?: (p: number) => void;
}

MyClass.prototype.extension = function (this: MyClass, p: number) {
    this.p = p;
    this.ppp = p;
};

const generic = function <T extends MyClass>(this: T, p: number) {
    this.p = p;
    this.ppp = p;
};

const typed = function (this: MyClass) {
    this.missing;
    const n: string = this.ppp;
};
