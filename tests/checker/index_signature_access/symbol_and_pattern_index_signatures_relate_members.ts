const sym = Symbol();
function gg3(x: { [key: string]: string }, y: { [key: symbol]: string }, z: { [sym]: number }, w: { [sym]: string }) {
    x = z;
    y = z;
    y = w;
}
type Pseudo = `&:${string}`;
type PseudoDeclaration = { [key in Pseudo]: string };
const ok: PseudoDeclaration = { '&:x' : 'someValue' };
type PathsObject = { [P in `/${string}`]: object; };
const pathObject: PathsObject = 123;
