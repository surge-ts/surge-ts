// @strict: true
// @surge-compare: messages
declare class Pool { query(): void }
declare class Client { connect(): void }
type NodeClient = Pool | Client;
declare function connect<C extends NodeClient = Pool>(...client: [C | string] | [C | string, number]): { client: NodeClient extends C ? Pool : C };
export const viaRest: 0 = connect("url");
declare function direct<C extends NodeClient = Pool>(client: C | string): { client: C };
export const viaUnion: 0 = direct("url");
declare function widened<T extends object = Date>(value: T | string): { value: T };
export const viaObject: 0 = widened("x");
