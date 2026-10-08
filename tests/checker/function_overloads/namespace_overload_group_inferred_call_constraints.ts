// @filename: node_modules/react/package.json
{ "name": "react", "types": "index.d.ts" }
// @filename: node_modules/react/index.d.ts
export = React; export as namespace React;
declare namespace React {
    type Dispatch<A> = (value: A) => void;
    type DispatchWithoutAction = () => void;
    type Reducer<S, A> = (prevState: S, action: A) => S;
    type ReducerWithoutAction<S> = (prevState: S) => S;
    type ReducerState<R extends Reducer<any, any>> = R extends Reducer<infer S, any> ? S : never;
    type ReducerAction<R extends Reducer<any, any>> = R extends Reducer<any, infer A> ? A : never;
    type ReducerStateWithoutAction<R extends ReducerWithoutAction<any>> = R extends ReducerWithoutAction<infer S> ? S
        : never;
    type DependencyList = readonly unknown[];

    function useReducer<R extends ReducerWithoutAction<any>, I>(
        reducer: R,
        initializerArg: I,
        initializer: (arg: I) => ReducerStateWithoutAction<R>,
    ): [ReducerStateWithoutAction<R>, DispatchWithoutAction];
    function useReducer<R extends ReducerWithoutAction<any>>(
        reducer: R,
        initializerArg: ReducerStateWithoutAction<R>,
        initializer?: undefined,
    ): [ReducerStateWithoutAction<R>, DispatchWithoutAction];
    function useReducer<R extends Reducer<any, any>, I>(
        reducer: R,
        initializerArg: I & ReducerState<R>,
        initializer: (arg: I & ReducerState<R>) => ReducerState<R>,
    ): [ReducerState<R>, Dispatch<ReducerAction<R>>];
    function useReducer<R extends Reducer<any, any>, I>(
        reducer: R,
        initializerArg: I,
        initializer: (arg: I) => ReducerState<R>,
    ): [ReducerState<R>, Dispatch<ReducerAction<R>>];



    function useReducer<R extends Reducer<any, any>>(
        reducer: R,
        initialState: ReducerState<R>,
        initializer?: undefined,
    ): [ReducerState<R>, Dispatch<ReducerAction<R>>];
}
// @filename: main.ts
import { useReducer } from 'react';
interface State { success: boolean }
type Action = { type: 'a' } | { type: 'b'; payload: true };
const reducer2 = (state: State, action: Action): State => state;
export const [s2, d2] = useReducer(reducer2, { success: false });
const p2: 1 = d2;
