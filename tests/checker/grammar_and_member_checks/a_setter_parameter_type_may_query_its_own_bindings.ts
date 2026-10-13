interface Named { name: string }
class C {
    set x({ name: alias }: Named & { o: typeof alias }) { }
}
interface I {
    set x({ name: alias }: Named & { o: typeof alias });
}
declare const i: I;
i.x = { name: "a", o: "b" };
i.x = { name: "a", o: 1 };
new C().x = { name: "a", o: "b" };
