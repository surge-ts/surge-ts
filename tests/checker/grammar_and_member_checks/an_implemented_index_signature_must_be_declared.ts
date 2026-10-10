// A class instance has no implicit index signature, so implementing an
// interface with one needs the class to declare it (TS2420); an inherited or
// declared index signature satisfies it.
interface Sig {
    [index: string]: string;
}
interface NumSig {
    [index: number]: string;
}
class Missing implements Sig {
}
class MissingNumber implements NumSig {
}
class Declared implements Sig {
    [index: string]: string;
}
class Base {
    [index: string]: string;
}
class Inherited extends Base implements Sig {
}
class StringCoversNumber implements NumSig {
    [index: string]: string;
}
