// Credit-line arithmetic, kept out of the page so the rule can be tested.
//
// A credit note's lines must account for the whole document: a detail that
// sums to a different figure than the face value is two answers to one
// question. The API refuses it, and the form should not send it.

export type CreditLineDraft = {
  source_kind: string;
  source_id: string;
  description: string;
  amount_minor: number;
  physical: boolean;
  disposition: string | null;
};

/** The sum of a draft's lines, in minor units. */
export function creditLinesTotal(lines: Pick<CreditLineDraft, "amount_minor">[]): number {
  return lines.reduce((sum, line) => sum + line.amount_minor, 0);
}

/** Whether the lines account for the document exactly. */
export function creditLinesMatchTotal(
  lines: Pick<CreditLineDraft, "amount_minor">[],
  totalMinor: number,
): boolean {
  return creditLinesTotal(lines) === totalMinor;
}

/** Whether a source id has the shape of a uuid, which every source kind uses. */
export function isUuid(value: string): boolean {
  return /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/.test(
    value.trim(),
  );
}
