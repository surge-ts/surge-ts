// @strict: true
// @experimentalDecorators: true
// A class's static initializers, static blocks and property decorators run
// while the class is defined, so a later enum or class is still in its
// temporal dead zone there (TS2450/TS2449), and a member read off a later
// enum, namespace or class is used before its initialization (TS2729).
// Instance initializers and method decorators run later or are not checked.
declare function deco(...args: any[]): any;

class Reader {
    static fromEnum = Later.No;
    static fromNamespace = Space.value;
    static fromClass = After.count;
    static fromElement = Later["No"];
    static deferred = () => Later.No;
    instance = Later.No;

    @deco(Later.No) decorated: number = 1;
    @deco(Space.value) static decoratedStatic: number = 1;
    @deco(Later.No) method() {}

    static {
        Later.No;
    }
}

enum Later { No }
namespace Space { export const value = 1; }
class After { static count = 1; }
