// Shared formatting helpers for Wave 9 staff surfaces.
export function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);
}
