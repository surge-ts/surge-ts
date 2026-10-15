// @experimentalDecorators: true
// @strict: false
interface Shaped<T> { prototype: T; m: () => T }
function generic<T>(c: Shaped<T>) {}
function exact(c: Shaped<Rejected>) {}
@generic
class Rejected { brand: any; static m() {} }
@exact
class AlsoRejected { brand: any; static m() {} }
function fromPrototype<T>(c: { prototype: T }) {}
@fromPrototype
class Accepted { x = 1; }
function anything(c: any) {}
@anything
class Untyped {}
function constructs(c: new () => { y: number }) {}
@constructs
class Constructs { y = 1; }
@constructs
class LacksMember { z = 1; }
