// InviteMemberFlow — the marking on the last step (Wave 68 lane
// `w68-f111-error-edge`).
//
// The load-bearing assertions are the MARKING, not the words alone. A bad name
// or a malformed address is answered on the field it belongs to: the input
// reports `aria-invalid`, its `aria-describedby` resolves to the message, and
// the other field stays clean. A refusal that names no field stays on the
// form-level line. And the submit button is never disabled into silence — the
// disabled-on-empty form is what made the empty-name and empty-email states
// unreachable in the first place, so its absence is a case of its own.
//
// This component owns no route: the POST belongs to the caller, which passes
// `send` in. The MockApi harness is installed all the same, and the suite
// asserts it recorded nothing, so a future edit that starts posting from here
// fails rather than quietly doubling the request.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  InviteMemberFlow,
  type InviteMemberBody,
  type InviteSendResult,
} from "../../components/onboarding/invite-member-flow";
import { MockApi, seedAuthTokens } from "../helpers/mock-api";

const NAME_MESSAGE = "Add this person's name so the invite has someone to address.";
const EMAIL_MISSING_MESSAGE = "Add their work email so the invite has somewhere to go.";
const EMAIL_SHAPE_MESSAGE =
  "That doesn't look like an email address. Use the form name@company.com.";
const FORM_MESSAGE = "Some details are missing or invalid. Check the form and try again.";

const NAME_ERROR_ID = "invite-member-name-error";
const EMAIL_ERROR_ID = "invite-member-email-error";

function renderFlow(send: (body: InviteMemberBody) => Promise<InviteSendResult>): {
  mock: MockApi;
  onSent: ReturnType<typeof vi.fn>;
} {
  seedAuthTokens();
  const mock = new MockApi().install();
  const onSent = vi.fn();
  render(
    <InviteMemberFlow
      send={send}
      onSent={onSent}
      testIds={{ name: "invite-name", email: "invite-email", send: "create-invite" }}
    />,
  );
  return { mock, onSent };
}

/** Walk the two intent/role steps to the name-and-email step. */
async function goToDetails(): Promise<void> {
  fireEvent.click(screen.getByRole("radio", { name: /Leads the workspace/ }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await screen.findByText("Who are you inviting?");
}

const nameInput = () => screen.getByLabelText("Display name");
const emailInput = () => screen.getByLabelText("Email");
const sendButton = () => screen.getByTestId("create-invite");

/** The text a screen reader would read as the control's description. Throws
 *  rather than returning null, so a missing wiring cannot pass an assertion
 *  about wiring. */
function descriptionOf(input: HTMLElement): string {
  const id = input.getAttribute("aria-describedby");
  if (id === null) throw new Error("control carries no aria-describedby");
  const node = document.getElementById(id);
  if (node === null) throw new Error(`aria-describedby "${id}" resolves to nothing`);
  return node.textContent ?? "";
}

const refusingSend = (message: string) => async (): Promise<InviteSendResult> => ({
  ok: false,
  message,
});

describe("InviteMemberFlow field marking", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("marks the name when it is empty and leaves the email alone", async () => {
    const { mock } = renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    fireEvent.change(emailInput(), { target: { value: "dana@acme.test" } });
    fireEvent.click(sendButton());
    await screen.findByText(NAME_MESSAGE);

    expect(nameInput()).toHaveAttribute("aria-invalid", "true");
    expect(nameInput()).toHaveAttribute("aria-describedby", NAME_ERROR_ID);
    expect(descriptionOf(nameInput())).toContain(NAME_MESSAGE);
    expect(emailInput()).not.toHaveAttribute("aria-invalid");
    expect(emailInput()).not.toHaveAttribute("aria-describedby");
    // Focus goes to the field that needs work, one frame after the message
    // lands, so the description is already in the DOM when it is announced.
    await waitFor(() => expect(nameInput()).toHaveFocus());
    // The client-side rejection never reached the caller's POST.
    expect(mock.calls).toHaveLength(0);
  });

  it("marks the email when the address is malformed and leaves the name alone", async () => {
    renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    fireEvent.change(nameInput(), { target: { value: "Dana" } });
    fireEvent.change(emailInput(), { target: { value: "not-an-email" } });
    fireEvent.click(sendButton());
    await screen.findByText(EMAIL_SHAPE_MESSAGE);

    expect(emailInput()).toHaveAttribute("aria-invalid", "true");
    expect(emailInput()).toHaveAttribute("aria-describedby", EMAIL_ERROR_ID);
    expect(descriptionOf(emailInput())).toContain(EMAIL_SHAPE_MESSAGE);
    expect(nameInput()).not.toHaveAttribute("aria-invalid");
    expect(nameInput()).not.toHaveAttribute("aria-describedby");
    await waitFor(() => expect(emailInput()).toHaveFocus());
  });

  it("marks both fields when both are wrong", async () => {
    renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    fireEvent.click(sendButton());
    await screen.findByText(NAME_MESSAGE);

    expect(nameInput()).toHaveAttribute("aria-invalid", "true");
    expect(descriptionOf(nameInput())).toContain(NAME_MESSAGE);
    expect(emailInput()).toHaveAttribute("aria-invalid", "true");
    expect(descriptionOf(emailInput())).toContain(EMAIL_MISSING_MESSAGE);
    // The first problem in reading order owns the cursor.
    await waitFor(() => expect(nameInput()).toHaveFocus());
  });

  it("clears a marked field as soon as it is typed in", async () => {
    renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    fireEvent.click(sendButton());
    await screen.findByText(NAME_MESSAGE);
    expect(document.getElementById(NAME_ERROR_ID)).not.toBeNull();

    fireEvent.change(nameInput(), { target: { value: "Dana" } });

    expect(nameInput()).not.toHaveAttribute("aria-invalid");
    expect(nameInput()).not.toHaveAttribute("aria-describedby");
    expect(document.getElementById(NAME_ERROR_ID)).toBeNull();
  });

  it("leaves a refusal that names no field on the form-level line", async () => {
    const { mock } = renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    fireEvent.change(nameInput(), { target: { value: "Dana" } });
    fireEvent.change(emailInput(), { target: { value: "dana@acme.test" } });
    fireEvent.click(sendButton());

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(FORM_MESSAGE);
    expect(nameInput()).not.toHaveAttribute("aria-invalid");
    expect(emailInput()).not.toHaveAttribute("aria-invalid");
    expect(document.getElementById(NAME_ERROR_ID)).toBeNull();
    expect(document.getElementById(EMAIL_ERROR_ID)).toBeNull();
    // The POST is the caller's: this component hands the body to `send` and
    // never issues a request of its own.
    expect(mock.calls).toHaveLength(0);
  });

  it("shows nothing before the first submit and keeps the send button reachable", async () => {
    renderFlow(refusingSend(FORM_MESSAGE));
    await goToDetails();

    // Both fields are empty here. A disabled button cannot be focused, cannot
    // be read out, and explains nothing — which is how these two states became
    // unreachable.
    expect(nameInput()).toHaveValue("");
    expect(emailInput()).toHaveValue("");
    expect(sendButton()).toBeEnabled();

    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.getElementById(NAME_ERROR_ID)).toBeNull();
    expect(document.getElementById(EMAIL_ERROR_ID)).toBeNull();
    expect(nameInput()).not.toHaveAttribute("aria-invalid");
    expect(emailInput()).not.toHaveAttribute("aria-invalid");
  });
});
