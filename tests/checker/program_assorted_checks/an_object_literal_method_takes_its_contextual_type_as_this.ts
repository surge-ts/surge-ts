// @strict: true
type WatchHandler<T> = (val: T) => void;
const w: Record<string, WatchHandler<any>> = {
    data1(val) { this.data2 = 1; },
    data2(val) {},
};
interface Opts { count: number; run(): void; }
const o: Opts = {
    count: 1,
    run() { this.count = "s"; this.missing; },
};
declare function take(o: { watch: Record<string, WatchHandler<any>> }): void;
take({ watch: { a(val) { this.b = 1; }, b(val) {} } });
const y: { m(): void } & ThisType<{ q: number }> = { m() { this.q = 1; } };
const plain = { x: 1, m() { this.x = 2; } };
export {};
