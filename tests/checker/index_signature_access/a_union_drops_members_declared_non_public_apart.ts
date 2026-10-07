// @surge-compare: spans
class Foo { protected foo = 0; }
class Bar { protected foo = 0; }
type _3 = (Foo & Bar)['foo'];
type _4 = (Foo | Bar)['foo'];
type _5 = (Foo | (Foo & Bar))['foo'];
type _6 = Foo['foo'];
