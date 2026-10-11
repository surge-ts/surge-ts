// tsc retries a rejected call against the overloads' implementation
// (`addImplementationSuccessElaboration`): an argument no candidate reached is
// contextually typed by the implementation's parameter, until the
// implementation rejects an argument too.
function pick(a: string, cb: (n: number) => void): void;
function pick(a: number, cb: (n: number) => void): void;
function pick(a: any, cb: any) {}
pick(true, z => { z.foo; });

function one(a: string, cb: (n: number) => void): void;
function one(a: any, cb: any) {}
one(1, y => { y.foo; });

function stops(a: string, b: string, cb: (n: number) => void): void;
function stops(a: any, b: number, cb: any) {}
stops(1, "x", v => { v.foo; });

declare function ambient(a: string, cb: (n: number) => void): void;
declare function ambient(a: number, cb: (n: number) => void): void;
ambient(true, w => { w.foo; });

function nested() {
    one(1, u => { u.foo; });
}

export {};
