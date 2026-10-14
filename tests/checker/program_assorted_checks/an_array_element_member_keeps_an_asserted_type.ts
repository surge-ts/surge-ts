// @strict: true
// An object literal element of an array literal widens each member at its own
// location: a fresh literal widens, while `"entry" as const` or a call
// returning a literal union keeps its type.
type NavIcon = "record" | "entry" | "rules";
type NavItem = { href: string; label: string; icon: NavIcon };
declare const officer: boolean;
export const items: NavItem[] = [
  { href: "/me", label: "a", icon: "record" },
  ...(officer ? [{ href: "/records", label: "b", icon: "entry" as const }] : []),
];
declare function icon(): NavIcon;
const listed = [{ icon: icon() }, { icon: "rules" as const }];
export const first: NavIcon = listed[0]!.icon;
const widened = [{ label: "a", count: 1 }];
widened.push({ label: "b", count: 2 });
