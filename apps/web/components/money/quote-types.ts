// The quote shapes the sales surfaces read, with the optional-line fields the
// API already persists but the generated SDK does not carry yet.
//
// Purpose: one place to widen the generated `Quote` / quote-line types while
// the OpenAPI pins catch up (the integration owner regenerates them; the SDK is
// generated code and is never hand-edited). Every field is declared optional
// here, so an SDK that gains them later narrows the intersection silently
// instead of breaking these call sites.
//
// Non-purpose: no arithmetic and no defaults — `draft-math` owns the preview
// and the server's roll-up owns the stored figures.

import type { components } from "@marula-core/sdk";

/** A quote document; `optional_minor` is the optional lines' total, never billed. */
export type QuoteRecord = components["schemas"]["Quote"] & { optional_minor?: number };

/** A persisted quote line; `optional` lines sit outside the total. */
export type QuoteLineRecord = components["schemas"]["QuoteLine"] & { optional?: boolean };

/** The quote document plus its lines, with both widened shapes. */
export type QuoteDetailRecord = Omit<components["schemas"]["QuoteDetail"], "lines"> & {
  optional_minor?: number;
  lines: QuoteLineRecord[];
};

/** A line as the create/patch routes accept it; `optional` defaults to false. */
export type QuoteLineDraft = components["schemas"]["QuoteLineInput"] & { optional?: boolean };

/**
 * Reads the persisted flag off a line the generated types do not widen yet.
 * One place for the cast, so no surface invents its own reading.
 */
export function isOptionalLine(line: components["schemas"]["QuoteLine"]): boolean {
  return (line as QuoteLineRecord).optional === true;
}
