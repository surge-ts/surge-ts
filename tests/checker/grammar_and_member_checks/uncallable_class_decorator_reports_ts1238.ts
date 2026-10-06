// @experimentalDecorators: true
class CtorDtor {}
@CtorDtor
class C {}
declare const notFn: number;
@notFn
class D {}
function ok(c: Function) {}
@ok
class E {}
