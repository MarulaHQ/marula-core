// Whether a register may say "nothing here".
//
// A failed read and an empty collection both leave zero rows on screen. They
// are not the same answer: "no vendor rentals in this view" under a load error
// tells the operator the opposite of what happened. The error is rendered
// beside the table, so the empty claim is only true when the read succeeded.

export function showsEmptyRegister(rowCount: number, loadError: string | null): boolean {
  return rowCount === 0 && loadError === null;
}
