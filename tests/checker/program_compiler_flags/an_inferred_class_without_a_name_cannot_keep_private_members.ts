// @declaration: true
// @target: es2015
// @module: commonjs
// @filename: mixins.ts
declare function mix<TMix>(mixin: TMix): TMix;

const DisposableMixin = class {
    protected _onDispose() {
        this._assertIsStripped()
    }
    private _assertIsStripped() {
    }
};

export default mix(DisposableMixin);
export class Monitor extends mix(DisposableMixin) {
    protected _onDispose() {
    }
}

// @filename: anonymous.ts
declare function mix<TMix>(mixin: TMix): TMix;
const Hidden = class {
    private secret() {}
};
export default class extends mix(Hidden) {
}

// @filename: functions.ts
export class FooItem {
    foo(): void { }
    name?: string;
    private property = "capitalism"
}
export type Constructor<T> = new(...args: any[]) => T;
export function WithTags<T extends Constructor<FooItem>>(Base: T) {
    return class extends Base {
        static getTags(): void { }
        tags(): void { }
    }
}
export class Test extends WithTags(FooItem) {}
export function local() {
    class Local { private hidden = 1; shown = 2 }
    return new Local();
}
export function named() {
    return FooItem;
}
export function annotated(): object {
    return class { private p = 1 };
}
function notExported() {
    return class { private p = 1 };
}
notExported();

// @filename: expressions.ts
declare function mix<TMix>(mixin: TMix): TMix;
export default mix(class { protected kept = 1 });
