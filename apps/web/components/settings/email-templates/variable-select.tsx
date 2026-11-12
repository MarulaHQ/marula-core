"use client";
/**
 * The variable-insertion dropdown shared by the template editor's subject
 * field and its rich-text body.
 *
 * Purpose: one control for "insert this template's placeholder at the
 * caret", drawn from the kind's closed vocabulary so a placeholder can
 * never be misspelled into a message. Tiptap-free on purpose — the body
 * editor leaf and the page both render it, and the lazy-boundary test
 * keeps heavy libraries out of anything but the leaf.
 */
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@marula-core/ui";
import { variableLabel } from "../../../lib/email-templates/variables";

export function VariableSelect({
  vocabulary,
  onInsert,
  ariaLabel,
}: {
  vocabulary: string[];
  onInsert: (key: string) => void;
  ariaLabel: string;
}): React.JSX.Element | null {
  if (vocabulary.length === 0) return null;
  return (
    <Select
      value=""
      onValueChange={(key) => {
        if (key !== "") onInsert(key);
      }}
    >
      <SelectTrigger className="w-47.5" aria-label={ariaLabel}>
        <SelectValue placeholder="Insert variable" />
      </SelectTrigger>
      <SelectContent>
        {vocabulary.map((key) => (
          <SelectItem key={key} value={key}>
            {variableLabel(key)}{" "}
            <span style={{ fontFamily: "var(--m-font-mono)" }}>{`{{${key}}}`}</span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
