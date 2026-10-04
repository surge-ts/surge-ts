// @experimentalDecorators: true
// @target: esnext
interface OmniDecorator extends MethodDecorator, ClassDecorator, PropertyDecorator {}

declare function allRest(...args: any[]): OmniDecorator;
declare function descriptorReturn(target: any, key: string, descriptor: PropertyDescriptor): PropertyDescriptor;
declare function voidReturn(target: any, key: string, descriptor: PropertyDescriptor): void;

class B {
    @allRest bar() { }
    @descriptorReturn baz() { }
    @voidReturn qux() { }
}
