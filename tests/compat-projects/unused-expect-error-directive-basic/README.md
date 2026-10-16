# unused-expect-error-directive-basic

An `@ts-expect-error` directive whose next non-blank, non-`//`-comment line has no error is TS2578 at the directive comment (the last line of a block comment); `@ts-ignore` never reports, and the directive must lead its comment (tsc's comment-directive processing).

surge reports TS2578 only in a file it checked with certainty (no generic or overloaded call inference, no generic instantiation from outside the default lib, no conditional, mapped or indexed access type, no degraded type), and never over a `type` or `interface` line; `SURGE_REPORT_UNUSED_EXPECT_ERROR=1` reports every one, as tsc does.
