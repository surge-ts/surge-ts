// The operand a condition tests for being an uncalled function is read after
// the `&&` operands before it, which hold there (TS2774); an optional method
// is still possibly undefined and is not reported.
declare const inBrowser: boolean;
declare const source: { mark(): void; measure(): void; clearMeasures(): void };
const perf = inBrowser && source;
if (perf && perf.mark && perf.measure && perf.clearMeasures) {
    perf.mark();
    perf.measure();
}
declare const optional: { method?: () => void } | undefined;
if (optional && optional.method) {
}
const conditional = perf && perf.clearMeasures ? 1 : 2;
