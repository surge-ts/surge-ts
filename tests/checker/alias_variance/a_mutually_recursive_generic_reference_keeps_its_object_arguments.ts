declare class StateNode<TContext, in out TEvent extends { type: string }> {
    _storedEvent: TEvent;
    _action: ActionObject<TEvent>;
}
interface ActionObject<TEvent extends { type: string }> {
    exec: (meta: StateNode<any, TEvent>) => void;
}
declare const play: ActionObject<{ type: "PLAY"; value: number }>;
const widened: ActionObject<{ type: "PLAY"; value: number } | { type: "RESET" }> = play;
declare const node: StateNode<any, { type: "PLAY"; value: number } | { type: "RESET" }>;
const narrowed: StateNode<any, { type: "PLAY"; value: number }> = node;
const exec: (meta: StateNode<any, { type: "PLAY"; value: number }>) => void = play.exec;
