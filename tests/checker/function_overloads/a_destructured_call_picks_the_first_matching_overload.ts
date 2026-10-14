// @strict: true
// Destructuring reads the call through the overload the arguments pick first.
type Anim = number | boolean | Record<string, number>;
interface Spring<T extends Anim> {
  setGoal(value: T): void;
}
declare function useSpring(initial: number): [number, Spring<number>];
declare function useSpring<T extends Anim>(initial: T): [T, Spring<T>];
const [, spring] = useSpring(0);
spring.setGoal(2);
declare function useObject(initial: number): { spring: Spring<number> };
declare function useObject<T extends Anim>(initial: T): { spring: Spring<T> };
const { spring: other } = useObject(0);
other.setGoal(2);
