// @noImplicitAny: true
import { Missing } from "missing-module";

declare function use(props: Omit<Missing, "as">): void;
use({ onClick: (e) => {} });

declare function keep(props: Omit<{ onClick(e: number): void; as: string }, "as">): void;
keep({ onClick: (e) => {} });
