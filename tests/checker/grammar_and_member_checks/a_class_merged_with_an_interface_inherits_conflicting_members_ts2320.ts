// @strict: true
// A class merged with an interface extends both of their bases; a member
// none of them declares that two bases give different types is TS2320, at
// the first interface declaration.
abstract class BaseClass {
  abstract bar: number;
}
class Broken extends BaseClass {}
interface IGetters {
  bar: number;
}
interface Broken extends IGetters {}
class IncorrectlyExtends extends BaseClass {}
interface IncorrectGetters {
  bar: string;
}
interface IncorrectlyExtends extends IncorrectGetters {}
