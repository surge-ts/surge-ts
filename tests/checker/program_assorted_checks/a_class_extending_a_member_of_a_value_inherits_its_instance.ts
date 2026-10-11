const UI = { TreeElement: class { treeOutline = 12 } };
class C extends UI.TreeElement {
    m() {
        this.doesNotExist;
        this.treeOutline.doesntExistEither();
        const n: number = this.treeOutline;
    }
}
