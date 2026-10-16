// @surge-compare: messages
interface Dict { [index: string]: number }
const d: Dict = 1;
const a: { [name: string]: string } = 1;
const b: string = { } as { [position: number]: boolean };
type Keys = { [K in string]: number };
const c: boolean = {} as Keys;
