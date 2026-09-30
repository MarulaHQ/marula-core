// Money-cluster grammar tests — the money surfaces' numbers and words, from
// the Marula surface pack's approvals reference (pack §3–§7). A wrong duration
// or a wrong colour meaning here costs real money, so the rules are pinned.
import { describe, expect, it } from "vitest";

import {
  ageingBucket,
  ageingLabel,
  dealPill,
  hms,
  invoicePill,
  looksFine,
  moneyItemFor,
  notFineReason,
  overBalanceMessage,
  payNote,
  quotePill,
  REJECT_REASONS,
  rejectReasonLabel,
} from "./money-ui";

describe("moneyItemFor", () => {
  it("files each route under its sub-nav item", () => {
    expect(moneyItemFor("/approvals")).toBe("approvals");
    expect(moneyItemFor("/billing")).toBe("invoices");
    expect(moneyItemFor("/billing/credit-notes")).toBe("credit-notes");
    expect(moneyItemFor("/sales/quotes")).toBe("quotes");
    expect(moneyItemFor("/sales/pipeline")).toBe("pipeline");
    expect(moneyItemFor("/sales/catalog")).toBe("catalogue");
  });

  it("keeps an invoice DETAIL inside Invoices, not under credit notes", () => {
    expect(moneyItemFor("/billing/40000000-0000-4000-8000-000000000001")).toBe("invoices");
  });

  it("claims nothing outside the cluster", () => {
    expect(moneyItemFor("/tickets")).toBeNull();
    expect(moneyItemFor("/")).toBeNull();
  });
});

describe("hms", () => {
  it("pads the hours and the minutes so the colons line up", () => {
    expect(hms(0)).toBe("0:00");
    expect(hms(45)).toBe("0:45");
    expect(hms(120)).toBe("2:00");
    expect(hms(1155)).toBe("19:15");
    expect(hms(840)).toBe("14:00");
  });

  it("rounds a partial minute and never goes negative", () => {
    expect(hms(44.6)).toBe("0:45");
    expect(hms(-30)).toBe("0:00");
    expect(hms(Number.NaN)).toBe("0:00");
  });
});

describe("ageingBucket", () => {
  const now = new Date("2026-09-29T11:41:00Z").getTime();

  it("calls a sent invoice with money owed past its date overdue", () => {
    expect(
      ageingBucket({ status: "issued", dueOn: "2026-09-17", unpaidMinor: 386000, now }),
    ).toBe("overdue");
  });

  it("keeps a due date inside three days due-soon, not overdue", () => {
    expect(ageingBucket({ status: "issued", dueOn: "2026-09-30", unpaidMinor: 386000, now })).toBe(
      "due-soon",
    );
    expect(ageingBucket({ status: "issued", dueOn: "2026-10-01", unpaidMinor: 386000, now })).toBe(
      "due-soon",
    );
  });

  it("leaves a date further out current", () => {
    expect(ageingBucket({ status: "issued", dueOn: "2026-10-20", unpaidMinor: 386000, now })).toBe(
      "current",
    );
  });

  it("settles a paid or void invoice regardless of its date", () => {
    expect(ageingBucket({ status: "paid", dueOn: "2026-01-01", unpaidMinor: 0, now })).toBe("settled");
    expect(ageingBucket({ status: "void", dueOn: "2026-01-01", unpaidMinor: 386000, now })).toBe(
      "settled",
    );
  });

  it("gives a draft no bucket: it is not owed anything yet", () => {
    expect(ageingBucket({ status: "draft", dueOn: "2026-09-01", unpaidMinor: 386000, now })).toBe(
      "none",
    );
  });

  it("survives a missing due date", () => {
    expect(ageingBucket({ status: "issued", dueOn: null, unpaidMinor: 386000, now })).toBe("none");
  });
});

describe("ageingLabel", () => {
  const now = new Date("2026-09-29T11:41:00Z").getTime();

  it("counts the days past due, in words", () => {
    expect(ageingLabel("2026-09-17", "overdue", now)).toBe("Overdue 12 days");
    expect(ageingLabel("2026-09-28", "overdue", now)).toBe("Overdue 1 day");
  });

  it("counts down to a due date", () => {
    expect(ageingLabel("2026-09-29", "due-soon", now)).toBe("Due today");
    expect(ageingLabel("2026-09-30", "due-soon", now)).toBe("Due tomorrow");
    expect(ageingLabel("2026-10-02", "current", now)).toBe("Due in 3 days");
  });

  it("says nothing when there is nothing to say", () => {
    expect(ageingLabel("2026-09-17", "settled", now)).toBeNull();
    expect(ageingLabel(null, "none", now)).toBeNull();
  });
});

describe("invoicePill", () => {
  it("reads Paid green with a check, Sent gold, Draft neutral", () => {
    expect(invoicePill("paid", 386000, 386000)).toEqual({
      label: "Paid",
      family: "green",
      icon: "check",
    });
    expect(invoicePill("issued", 0, 386000).family).toBe("gold");
    expect(invoicePill("draft", 0, 386000).family).toBe("neutral");
  });

  it("distinguishes a part-paid invoice from a fully sent one", () => {
    expect(invoicePill("issued", 100000, 386000).label).toBe("Part paid");
    expect(invoicePill("issued", 0, 386000).label).toBe("Sent");
  });
});

describe("quotePill and dealPill", () => {
  it("keeps the quote lifecycle on the same pill rules", () => {
    expect(quotePill("draft").family).toBe("neutral");
    expect(quotePill("sent").family).toBe("gold");
    expect(quotePill("accepted")).toEqual({ label: "Accepted", family: "green", icon: "check" });
    expect(quotePill("declined").label).toBe("Declined");
    expect(quotePill("expired").label).toBe("Expired");
  });

  it("names a closed deal without a colour alarm", () => {
    expect(dealPill("won").family).toBe("green");
    expect(dealPill("lost").family).toBe("neutral");
    expect(dealPill("open").family).toBe("neutral");
  });
});

describe("looksFine", () => {
  const good = {
    own: false,
    minutes: 120,
    notes: "Replaced the failed drive and confirmed the array rebuilt cleanly.",
  };

  it("passes an ordinary entry", () => {
    expect(looksFine(good)).toBe(true);
  });

  it("never passes your own time", () => {
    expect(looksFine({ ...good, own: true })).toBe(false);
    expect(notFineReason({ ...good, own: true })).toContain("your own time");
  });

  it("never passes an entry the audit flagged, once it has been granted", () => {
    expect(looksFine({ ...good, flagged: true })).toBe(false);
  });

  it("needs a real description, not two words", () => {
    expect(looksFine({ ...good, notes: "Onsite support" })).toBe(false);
    expect(looksFine({ ...good, notes: null })).toBe(false);
  });

  it("needs four hours or less", () => {
    expect(looksFine({ ...good, minutes: 240 })).toBe(true);
    expect(looksFine({ ...good, minutes: 245 })).toBe(false);
  });

  it("always has a reason to give", () => {
    expect(notFineReason(good)).toBe("It is not in the pass.");
    expect(notFineReason({ ...good, notes: "Onsite support" })).toContain("not enough written");
    expect(notFineReason({ ...good, minutes: 300 })).toContain("5:00");
  });
});

describe("payNote", () => {
  // The pack's money grammar: grouped thousands, two decimals.
  const fmt = (minor: number): string =>
    `$${(minor / 100).toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })}`;

  it("says the invoice closes when the balance is covered", () => {
    const note = payNote(386000, 386000, fmt);
    expect(note.settles).toBe(true);
    expect(note.text).toContain("marked Paid");
    expect(note.text).toContain("reversed");
  });

  it("spells out the remaining balance when the payment is short", () => {
    const note = payNote(100000, 386000, fmt);
    expect(note.settles).toBe(false);
    expect(note.text).toContain("Balance after this payment: $2,860.00");
    expect(note.text).toContain("stays open");
  });
});

describe("overBalanceMessage", () => {
  const fmt = (minor: number): string =>
    `$${(minor / 100).toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })}`;

  it("blocks an amount over the balance and says what to do instead", () => {
    expect(overBalanceMessage(400000, 386000, fmt)).toBe(
      "That is more than the balance of $3,860.00. Record the balance, then a credit for the rest.",
    );
  });

  it("asks for an amount when there is none", () => {
    expect(overBalanceMessage(0, 386000, fmt)).toBe("Enter the amount received.");
  });

  it("lets a partial payment through", () => {
    expect(overBalanceMessage(386000, 386000, fmt)).toBeNull();
    expect(overBalanceMessage(1000, 386000, fmt)).toBeNull();
  });
});

describe("reject reasons", () => {
  it("carries a plain-English label on every typed code", () => {
    for (const reason of REJECT_REASONS) {
      expect(reason.code).not.toBe("");
      expect(reason.label).not.toBe("");
    }
  });

  it("resolves a code back to its label and passes an unknown one through", () => {
    expect(rejectReasonLabel("missing_notes")).toBe("Description is too vague");
    expect(rejectReasonLabel("brand_new_code")).toBe("brand_new_code");
    expect(rejectReasonLabel(null)).toBe("Rejected");
  });
});
