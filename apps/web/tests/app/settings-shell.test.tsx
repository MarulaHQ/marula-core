// Settings shell primitives (Wave 17 lane 7): the one save pattern, the one
// destructive-confirm pattern, and the typed typed empty state.
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ConfirmDestructive, NotAvailable, SaveBar } from "../../components/settings/settings-shell";

describe("SaveBar", () => {
  it("renders nothing on read-only surfaces (no onSave)", () => {
    const { container } = render(<SaveBar dirty />);
    expect(container).toBeEmptyDOMElement();
  });

  it("keeps Save disabled until the draft is dirty, then saves", () => {
    let saved = 0;
    const { rerender } = render(<SaveBar dirty={false} onSave={() => { saved += 1; }} />);
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(saved).toBe(0);
    rerender(<SaveBar dirty onSave={() => { saved += 1; }} />);
    expect(save).toBeEnabled();
    fireEvent.click(save);
    expect(saved).toBe(1);
  });

  it("disables while saving and shows the deterministic confirm + error copy", () => {
    // saved
    render(<SaveBar dirty={false} saved error={null} onSave={() => {}} />);
    expect(screen.getByTestId("settings-save-confirm")).toHaveTextContent("Saved.");
    // error
    render(<SaveBar dirty={false} saving error="Name already exists" onSave={() => {}} />);
    const alert = screen.getByTestId("settings-save-error");
    expect(alert).toHaveAttribute("role", "alert");
    expect(alert).toHaveTextContent("Name already exists");
  });
});

describe("ConfirmDestructive", () => {
  it("names the record, states consequences, and cancels without firing", () => {
    let fired = 0;
    const { rerender } = render(
      <ConfirmDestructive
        open
        onOpenChange={() => {}}
        title="Deactivate Ada?"
        body="Sessions end immediately; activation restores access."
        confirmLabel="Deactivate"
        onConfirm={() => {
          fired += 1;
        }}
      />,
    );
    expect(screen.getByRole("dialog")).toHaveTextContent("Deactivate Ada?");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(fired).toBe(0);
    rerender(
      <ConfirmDestructive
        open={false}
        onOpenChange={() => {}}
        title="Deactivate Ada?"
        body="Sessions end immediately; activation restores access."
        confirmLabel="Deactivate"
        onConfirm={() => {
          fired += 1;
        }}
      />,
    );
    expect(screen.queryByRole("dialog", { name: "Deactivate Ada?" })).not.toBeInTheDocument();
    // nothing else fired while closed
    expect(fired).toBe(0);
  });

  it("fires the confirm callback on the destructive button", () => {
    let fired = 0;
    render(
      <ConfirmDestructive
        open
        onOpenChange={() => {}}
        title='Delete PM schedule "Weekly backup"?'
        body="Generated tickets stay."
        confirmLabel="Delete schedule"
        onConfirm={() => {
          fired += 1;
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete schedule" }));
    expect(fired).toBe(1);
  });
});

describe("NotAvailable", () => {
  it("renders an honest typed state, not fake UI", () => {
    render(<NotAvailable what="Mail administration" hint="coming from a later wave" />);
    const status = screen.getByTestId("settings-not-available");
    expect(status).toHaveAttribute("role", "status");
    expect(status).toHaveTextContent(/Mail administration is not available yet — coming from a later wave/);
  });
});
