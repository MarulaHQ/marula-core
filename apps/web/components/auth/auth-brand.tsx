// Auth-side brand header shared by the public auth surfaces (/login,
// /mfa, /sso, /accept-invite).
//
// The lockup is the approved Marula wordmark from the brand kit: ink on a
// light surface, cream on a dark one, wordmark lowercase and outlined in the
// SVG so no font install is needed. The size passed here clears the kit's
// 96px minimum lockup width — see LOCKUP_COMPLIANT_HEIGHT in ../brand, which
// also degrades to the mark if a caller ever asks for less.
"use client";

import { BrandLockup, LOCKUP_COMPLIANT_HEIGHT } from "../brand";

export function AuthBrand({ subtitle }: { subtitle?: string }) {
  return (
    <div className="grid justify-items-center gap-2 text-center">
      <BrandLockup size={LOCKUP_COMPLIANT_HEIGHT} />
      {subtitle !== undefined && (
        <p className="max-w-[36ch] text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {subtitle}
        </p>
      )}
    </div>
  );
}
