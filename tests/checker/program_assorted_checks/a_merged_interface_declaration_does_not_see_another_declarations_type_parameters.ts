interface i10 { x: T; }
interface i10<T = number> {}

interface i11<T> { y: T; }
interface i11 { z: T; }

interface i12 { m<T>(t: T): T; }
interface i12<T = string> {}

type T13 = number;
interface i13 { w: T13; }
interface i13<T13 = string> {}
