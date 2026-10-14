// @strict: true
// `unknown | undefined` read off an optional member is `unknown`, which an
// equality test narrows to the compared literal.
type Theme = "light" | "dark";
declare function apply(theme: Theme): void;
export function onMessage(data: { theme?: unknown } | null) {
  if (data === null) return;
  if (data.theme === "light" || data.theme === "dark") apply(data.theme);
  apply(data.theme);
}
