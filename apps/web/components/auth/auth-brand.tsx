// Auth-side brand header shared by the public auth surfaces (/login,
// /mfa, /sso, /accept-invite): the marula-ci logo lockup (logo bundle,
// Wave 17 lane 10) replaces the placeholder mark — same navy/white variants
// as the header, theme-aware. marula-ci/source/*.svg stays the editable
// source of truth; the auth cover display title stays Inter per the
// 2026-09-18 diktat (Source Serif remains AuthBrand wordmark-only in the
// docs), rendered in the lockup image itself.
"use client";

import { BrandLockup } from "../brand";

export function AuthBrand({ subtitle }: { subtitle?: string }) {
  return (
    <div className="grid justify-items-center gap-2 text-center">
      <BrandLockup size={22} />
      {subtitle !== undefined && (
        <p className="max-w-[36ch] text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {subtitle}
        </p>
      )}
    </div>
  );
}