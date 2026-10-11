type M = { [K in keyof Missing1]: Missing2 };
type N<P> = { [K in keyof Missing3]: P extends Missing4[K] ? K : never };
namespace NS { export interface X { a: string } }
type Q<P> = { [K in keyof NS.Y]: P extends NS.Z[K] ? K : never };
type Keyed = { [K in "a"]: Missing5 };
