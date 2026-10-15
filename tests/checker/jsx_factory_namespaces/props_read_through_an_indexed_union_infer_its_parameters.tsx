// @jsx: preserve
// @noImplicitAny: true
interface Options<State, Actions> {
    state?: State;
    actions: string | Actions;
}
namespace JSX {
    export interface Element {}
    export interface IntrinsicElements {}
}
interface ActionsObjectOr<State> {
    [prop: string]: ((state: State) => State) | State;
}
declare function App<State, Actions extends ActionsObjectOr<State>>(
    props: Options<State, Actions>["actions"] & { state: State },
): JSX.Element;
const element = <App state={100} foo={s => s} />;
const called = App({ state: 100, foo: (s: number) => s });

declare function Direct<State, Actions extends ActionsObjectOr<State>>(
    props: (string | Actions) & { state: State },
): Actions;
const actions: number = Direct({ state: 100, foo: (s: number) => s });
