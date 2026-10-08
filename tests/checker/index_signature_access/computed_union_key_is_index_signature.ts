interface Student { id: string; name: string; age: number; email: string }
type EditableField = "name" | "age";
declare function update(data: Partial<Student>): void;
export function f<Field extends EditableField>(id: string, field: Field, value: Student[Field]) {
  update({ id, [field]: value });
}
export function g(id: string, field: EditableField, value: string) {
  update({ id, [field]: value });
}
export function h(id: string, field: "name", value: string) {
  update({ id, [field]: value });
  update({ id, [field]: 1 });
}
