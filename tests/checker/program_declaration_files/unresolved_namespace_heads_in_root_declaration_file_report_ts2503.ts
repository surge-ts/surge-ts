// @filename: a.d.ts
declare namespace lf {
    export interface Transaction {
        attach(query: query.Builder): void;
        x: Missing;
        y: lf.Transaction;
    }
}
declare var w: ns2.T;
interface Top { m(a: ns3.T): void }
