// Unit test: the settings form family's error copy. A field error reports
// that a value did not land, so it wears the status danger pair. The due pair
// stays for dates that are coming up, and the message never leans on colour
// alone (docs/theme.md §3.4).
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { FieldInput, FormField } from "../../components/settings/settings-forms";

describe("settings form family", () => {
  it("marks a field error with the status danger pair, never the due pair", () => {
    render(
      <FormField
        label="Slack webhook"
        htmlFor="hook"
        error="That URL did not answer."
        testId="hook"
      >
        <FieldInput id="hook" value="" onChange={() => {}} invalid />
      </FormField>,
    );

    const note = screen.getByRole("alert");
    expect(note).toHaveTextContent("That URL did not answer.");
    // The declaration, not the computed value: jsdom does not resolve
    // `var(...)` against the theme's custom properties, so `toHaveStyle`
    // reads the initial colour and fails on a page that paints correctly.
    expect(note.style.color).toBe("var(--m-status-danger-fg)");
    expect(note.style.color).not.toBe("var(--m-status-due-fg)");
    // Not colour alone: the alert glyph travels with the sentence.
    expect(note.querySelector("svg")).not.toBeNull();
  });
});
