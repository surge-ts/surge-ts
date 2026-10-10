// A function argument infers through an interface target's call signature
// (`inferFromSignatures`): `P` is the component's props, which the props
// argument is then checked against.
interface ExactProps {
    value: "A" | "B";
}
interface FunctionComponent<P = {}> {
    (props: P): null;
}
declare class Component<P> {
    constructor(props: P);
}
interface ComponentClass<P = {}> {
    new (props: P): Component<P>;
}
type Ctor<P> = ((props: P) => null) | (new (props: P) => Component<any>);
declare function create<P extends {}>(
    type: FunctionComponent<P> | ComponentClass<P> | string,
    props?: P | null,
): void;
declare let component: Ctor<ExactProps>;
create(component, { value: "C" });
declare let plain: (props: ExactProps) => null;
create(plain, { value: "C" });
create(plain, { value: "A" });
