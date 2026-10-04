interface Animal { animal: string }
interface Dog extends Animal { dog: string }

interface Comparer<T> {
    compare: (a: T, b: T) => number;
}
declare let animalComparer: Comparer<Animal>;
declare let dogComparer: Comparer<Dog>;
animalComparer = dogComparer;
dogComparer = animalComparer;

interface Crate<T> {
    item: T;
    onSetItem: (item: T) => void;
}
declare let animalCrate: Crate<Animal>;
declare let dogCrate: Crate<Dog>;
animalCrate = dogCrate;
dogCrate = animalCrate;

interface MethodOnly<T> {
    take(item: T): void;
}
declare let animalMethod: MethodOnly<Animal>;
declare let dogMethod: MethodOnly<Dog>;
animalMethod = dogMethod;
dogMethod = animalMethod;

type Handler<T> = (value: T) => void;
declare let animalHandler: Handler<Animal>;
declare let dogHandler: Handler<Dog>;
animalHandler = dogHandler;
dogHandler = animalHandler;
