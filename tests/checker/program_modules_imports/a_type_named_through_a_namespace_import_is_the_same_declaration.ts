// @surge-compare: messages
// @filename: box.ts
export interface Box<T> { value: T }

// @filename: index.ts
import * as boxes from "./box";
import { Box } from "./box";

declare const either: boxes.Box<string> | Box<string>;
const count: number = either;

declare const pair: [boxes.Box<string>, Box<string>];
const counts: [number, number] = pair;
