// @strict: true
// An abstract construct signature survives a mixin intersection: `new` on it
// is TS2511 and a concrete subclass owes its abstract members (TS2515); a
// class extending a type variable constrained to one must be abstract (TS2797).
interface Mixin {
    mixinMethod(): void;
}
function Mixin<TBaseClass extends abstract new (...args: any) => any>(baseClass: TBaseClass): TBaseClass & (abstract new (...args: any) => Mixin) {
    // error expected: A mixin class that extends from a type variable containing an abstract construct signature must also be declared 'abstract'.
    class MixinClass extends baseClass implements Mixin {
        mixinMethod() {
        }
    }
    return MixinClass;
}
abstract class AbstractBase {
    abstract abstractBaseMethod(): void;
}
const MixedBase = Mixin(AbstractBase);
// error expected: Non-abstract class 'DerivedFromAbstract' does not implement inherited abstract member 'abstractBaseMethod' from class 'AbstractBase & Mixin'.
class DerivedFromAbstract extends MixedBase {
}
// error expected: Cannot create an instance of an abstract class.
new MixedBase();
export {};
