# overload-predicate-callback-basic

`compareSignaturesRelated`: a parameter typed by an identifier type-predicate
signature takes only a callback that is one — written (`(v): v is T => …`)
or read from its body (`getTypePredicateFromBody`). So zod's
`refine<R extends Output>(check: (arg: Output) => arg is R)` is passed over
for `(val) => val.length > 5` and the `(arg: Output) => unknown` overload
types the call, while a written predicate infers `R` from its type. Chained
`this`-returning wrappers then keep their real arguments.
