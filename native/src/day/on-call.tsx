// On-call awareness — the one strip that says whether the phone is the
// one that rings.
//
// Purpose: read the technician's own rotation blocks (today plus two
// weeks) and answer two questions in a glance: on call right now, or when
// the next shift starts. The strip renders nothing when the window holds
// no rotation, so a normal week stays quiet.
//
// Non-purpose: no rotation editing (dispatchers own the schedule), no
// pager integration.

import { StyleSheet, Text, View } from "react-native";

import type { OnCallBlock } from "@/core/models";
import { Pill } from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export interface OnCallStripProps {
  blocks: OnCallBlock[];
  now: number;
}

export function OnCallStrip({ blocks, now }: OnCallStripProps) {
  const { tokens } = useTheme();
  const active = blocks.find(
    (block) => Date.parse(block.starts_at) <= now && now < Date.parse(block.ends_at),
  );
  const next = active ?? blocks.find((block) => Date.parse(block.starts_at) > now);
  if (next === undefined) return null;

  const label = active !== undefined ? "ON CALL" : "UPCOMING ON CALL";
  const range = active !== undefined
    ? `until ${shortStamp(active.ends_at)}`
    : `${shortStamp(next.starts_at)} – ${shortStamp(next.ends_at)}`;

  return (
    <View
      testID="on-call-strip"
      style={[
        styles.strip,
        {
          backgroundColor: active !== undefined ? tokens.danger.background : tokens.surface,
          borderColor: active !== undefined ? tokens.danger.text : tokens.border,
        },
      ]}
    >
      <Pill
        pair={active !== undefined ? tokens.danger : tokens.warning}
        label={label}
      />
      <View style={styles.copy}>
        <Text style={[styles.category, { color: tokens.text }]}>
          {titleCase(next.category)} rotation
        </Text>
        <Text style={[styles.range, { color: tokens.textSecondary }]}>{range}</Text>
      </View>
    </View>
  );
}

function titleCase(value: string): string {
  if (value.length === 0) return value;
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function shortStamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const day = date.toLocaleDateString(undefined, { weekday: "short" });
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(
    date.getMinutes(),
  ).padStart(2, "0")}`;
  return `${day} ${time}`;
}

const styles = StyleSheet.create({
  strip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  copy: {
    flex: 1,
    gap: 2,
  },
  category: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  range: {
    fontFamily: type.mono,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
});
