// The warranty facts as the API carries them, and the words a surface puts
// around them.
//
// The horizon and the window are the SERVER's (WarrantyWindow, derived once
// from an asset row through the shared domain rule in core-domain). What a
// surface owns is the sentence: the native found sheet prints a pill, the web
// ledger prints a column. Two composers over one set of facts, rather than two
// surfaces each re-deriving the horizon — which is how a sheet and a ledger
// end up disagreeing about which side of the window an asset sits on.

/** Exactly the payload's warranty shape, minus the SDK's optional wrapper. */
export type WarrantyHorizon = {
  state: "active" | "ending" | "ended";
  ends_on: string;
  days_remaining: number;
};

/** The found sheet's pill sentence, in the register the pack draws it:
 * "Ends in 41 days" on a clock. Ended counts backwards, covered carries its
 * date, and a missing horizon returns `null` so the caller can say "Not set"
 * rather than invent a date. */
export function warrantySentence(warranty: WarrantyHorizon): string {
  const days = Math.round(warranty.days_remaining);
  if (warranty.state === "ended") {
    const ago = Math.abs(days);
    return ago === 0 ? "Ended today" : `Ended ${ago} days ago`;
  }
  if (warranty.state === "ending") {
    return days === 0 ? "Ends today" : `Ends in ${days} days`;
  }
  return `Covered until ${warranty.ends_on}`;
}

/** Which status pair the pill wears: attention while it is ending, danger
 * once it has lapsed, success while it is still covered. Red is reserved for
 * a horizon that has actually passed, never for one that is merely close. */
export function warrantyPairKey(
  warranty: WarrantyHorizon,
): "warning" | "danger" | "success" {
  if (warranty.state === "ended") return "danger";
  if (warranty.state === "ending") return "warning";
  return "success";
}
