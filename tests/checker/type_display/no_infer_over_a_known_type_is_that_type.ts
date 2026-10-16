// @surge-compare: messages
interface Dog { woof(): void }
interface Animal { name: string }
declare let an: Animal;
const d: NoInfer<Dog> = an;
declare function assertEqual<T>(actual: T, expected: NoInfer<T>): boolean;
const g = { x: 3, y: 2 };
assertEqual(g, { x: 3 });
