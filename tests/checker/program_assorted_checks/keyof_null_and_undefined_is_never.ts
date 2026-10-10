// `null`, `undefined` and `void` have no properties, so they have no keys.
function keyed<T>(target: T, key: keyof T) {
    return target[key];
}
keyed(null, "foo");
keyed(undefined, "foo");
type NullKeys = keyof null;
const nullKey: NullKeys = "foo";
type UndefinedKeys = keyof undefined;
const undefinedKey: UndefinedKeys = "foo";
type VoidKeys = keyof void;
const voidKey: VoidKeys = "foo";
