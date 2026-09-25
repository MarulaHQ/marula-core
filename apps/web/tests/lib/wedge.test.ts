// Keyboard-wedge detector rules (Wave 32 lane 5, `p32-labels-scanning`).
//
// The detector is the heart of the scan bar: a burst of printable keys with
// sub-threshold gaps, ended by Enter, is a scan. Humans typing (variable
// gaps), broken bursts, modifier chords and short blips are not — the bar
// must never route a human's typing as a barcode.
import { describe, expect, it } from "vitest";
import {
  burstLandedInForeignField,
  createWedgeState,
  feedWedge,
  stripBurstFromField,
  WEDGE_MAX_GAP_MS,
} from "../../lib/scan/wedge";

function burst(keys: string, startAt = 1_000, gap = 12) {
  let state = createWedgeState();
  let at = startAt;
  let result: ReturnType<typeof feedWedge>["result"] = { kind: "ignored" };
  for (const key of keys) {
    const step = feedWedge(state, { key, at });
    state = step.next;
    result = step.result;
    at += gap;
  }
  return { state, at, lastResult: result };
}

describe("feedWedge", () => {
  it("detects a scanner burst terminated by Enter", () => {
    const { state, at } = burst("C6-1M");
    const done = feedWedge(state, { key: "Enter", at: at + 10 });
    expect(done.result).toEqual({ kind: "scan", text: "C6-1M" });
    expect(done.next.buffer).toBe("");
  });

  it("treats the deep-link payload (colons and slashes) as plain characters", () => {
    const { state, at } = burst("meridian://stock-item/abc");
    const done = feedWedge(state, { key: "Enter", at: at + 8 });
    expect(done.result).toEqual({ kind: "scan", text: "meridian://stock-item/abc" });
  });

  it("does not scan slow human typing", () => {
    // 180 ms between keys is typing, not a wedge.
    const { state, at } = burst("NOPE", 1_000, 180);
    const done = feedWedge(state, { key: "Enter", at: at + 150 });
    expect(done.result).toEqual({ kind: "reset" });
  });

  it("does not scan a one-character blip", () => {
    const step = feedWedge(createWedgeState(), { key: "x", at: 1_000 });
    const done = feedWedge(step.next, { key: "Enter", at: 1_010 });
    expect(done.result).toEqual({ kind: "reset" });
  });

  it("a gap mid-burst restarts the buffer (only the tail can scan)", () => {
    let state = createWedgeState();
    state = feedWedge(state, { key: "A", at: 1_000 }).next;
    state = feedWedge(state, { key: "B", at: 1_010 }).next;
    // Long pause then three fast keys: the tail is the scan.
    state = feedWedge(state, { key: "C", at: 2_000 }).next;
    state = feedWedge(state, { key: "D", at: 2_010 }).next;
    state = feedWedge(state, { key: "E", at: 2_020 }).next;
    const done = feedWedge(state, { key: "Enter", at: 2_030 });
    expect(done.result).toEqual({ kind: "scan", text: "CDE" });
  });

  it("modifier chords break the burst and are never payload", () => {
    let state = feedWedge(createWedgeState(), { key: "A", at: 1_000 }).next;
    const chord = feedWedge(state, { key: "b", at: 1_010, ctrlKey: true });
    expect(chord.result).toEqual({ kind: "reset" });
    expect(chord.next.buffer).toBe("");
  });

  it("non-printable keys end a burst rather than joining it", () => {
    let state = feedWedge(createWedgeState(), { key: "A", at: 1_000 }).next;
    const tab = feedWedge(state, { key: "Tab", at: 1_010 });
    expect(tab.result).toEqual({ kind: "reset" });
  });

  it("caps absurdly long bursts (a stuck key is not a scan)", () => {
    let state = createWedgeState();
    let at = 1_000;
    let sawReset = false;
    let maxBuffer = 0;
    for (let i = 0; i < 600; i += 1) {
      const step = feedWedge(state, { key: "x", at });
      state = step.next;
      if (step.result.kind === "reset") sawReset = true;
      maxBuffer = Math.max(maxBuffer, state.buffer.length);
      at += 5;
    }
    expect(sawReset).toBe(true);
    expect(maxBuffer).toBeLessThanOrEqual(512);
    expect(state.buffer.length).toBeLessThan(600);
  });

  it("threshold constant is the documented 55 ms", () => {
    expect(WEDGE_MAX_GAP_MS).toBe(55);
  });
});

describe("field hygiene", () => {
  it("recognises a foreign field (but not the scan input itself)", () => {
    const scanInput = document.createElement("input");
    const other = document.createElement("input");
    const div = document.createElement("div");
    expect(burstLandedInForeignField(other, scanInput)).toBe(true);
    expect(burstLandedInForeignField(scanInput, scanInput)).toBe(false);
    expect(burstLandedInForeignField(div, scanInput)).toBe(false);
  });

  it("strips exactly the injected burst from the field's tail", () => {
    const field = document.createElement("input");
    field.value = "user text C6-1M";
    stripBurstFromField(field, "C6-1M");
    expect(field.value).toBe("user text ");
  });

  it("leaves the field alone when the burst is not its tail", () => {
    const field = document.createElement("input");
    field.value = "untouched";
    stripBurstFromField(field, "ZZZ");
    expect(field.value).toBe("untouched");
  });
});
