// Auth-side brand header shared by the public auth surfaces (/login,
// /mfa, /sso, /accept-invite): the Meridian wordmark with the §7 mark
// direction — meridian line crossing a globe wireframe, one tangerine
// stroke — rendered entirely in currentColor + var(--m-accent) so the
// hex-scan surface stays clean and both modes inherit parity.
"use client";

export function AuthBrand({ subtitle }: { subtitle?: string }) {
  return (
    <div className="grid justify-items-center gap-2 text-center">
      <span className="flex items-center gap-2.5" aria-hidden="true">
        <svg width="26" height="26" viewBox="0 0 26 26" fill="none">
          <circle cx="13" cy="13" r="11" stroke="var(--m-border-strong)" strokeWidth="1.5" />
          <ellipse cx="13" cy="13" rx="5" ry="11" stroke="var(--m-border-strong)" strokeWidth="1.5" />
          <path d="M2 13h22" stroke="var(--m-accent)" strokeWidth="2" strokeLinecap="round" />
        </svg>
      </span>
      <span className="font-heading text-lg font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
        Meridian
      </span>
      {subtitle !== undefined && (
        <p className="max-w-[36ch] text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {subtitle}
        </p>
      )}
    </div>
  );
}