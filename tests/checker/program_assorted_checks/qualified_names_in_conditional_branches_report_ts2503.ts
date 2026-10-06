type A<It> = It extends NS.X ? NS.Y<It> : NS.Z;
type C<T> = T extends string ? Q.R : Q.S;
type E<T> = T extends string ? W.A : number;
namespace Known { export type T = number; }
type F<T> = T extends string ? Known.T : Known.T;
