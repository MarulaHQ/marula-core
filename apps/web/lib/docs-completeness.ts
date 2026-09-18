// Documentation completeness (Wave 17 lane 2 → lane 13).
//
// Scoring moved server-side: GET /api/v1/clients/{id}/completeness owns
// the weights and the pro-rating; the client page renders the backend
// view verbatim and honest "not available" states. This module now only
// keeps the score-tone mapping the pill uses (AA pairs ride the
// semantic status tokens).

/** Tone for the score pill (AA pairs ride the semantic status tokens). */
export function scoreTone(score: number): "success" | "warning" | "danger" {
  if (score >= 75) return "success";
  if (score >= 40) return "warning";
  return "danger";
}
