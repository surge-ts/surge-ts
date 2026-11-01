// @surge-compare: messages
interface El { something: any }
interface A { x: number }
interface B { y: string }

declare const ab: A & B;
const nullable: El | null = ab;

declare function take(e: El | null): void;
take(ab);

declare function optional(e?: El): void;
optional(ab);

declare const holder: { a: A & B };
const optionalProperty: { a?: El } = holder;

declare const overlapping: A & { something?: string };
const overlap: El | null = overlapping;

const weak: { a?: number } | undefined = ab;

declare const plain: { x: number };
const control: El | null = plain;
