// @strict: true
namespace ts {
    class Type {
        getProperty(index: number): Property {
            return undefined;
        }
        getSignature(index: number): Signature {
            return null;
        }
    }
    class Property {
        constructor(public name: string, public type: Type) { }
    }
    class Signature {
        constructor(public typeParameters: TypeParameter[], public returnType: Type) { }
    }
    const p: Property = undefined;
}
