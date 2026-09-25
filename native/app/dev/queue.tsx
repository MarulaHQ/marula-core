// Offline queue inspector (dev tool + honest surface for rejected writes).
//
// Purpose: every queued write shows its entity, attempts, and full audit
// trail (enqueued → sent → retry/applied/conflict_resolved/rejected). A
// rejected write is never silently dropped: a human retries it (after
// fixing the cause) or discards it, and that choice is recorded.

import { router } from "expo-router";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { QueuedOp } from "@/core/queue";
import { useEngine, useQueue } from "@/data/sync-context";
import { Button, Card, EmptyState, Mono, Pill, Screen, SectionHeader } from "@/ui/primitives";
import { accentPair, type, useTheme } from "@/ui/tokens";

export default function QueueInspector() {
  const { tokens } = useTheme();
  const { pending, rejected, pendingCount } = useQueue();
  const engine = useEngine();

  return (
    <Screen testID="queue-screen">
      <View style={styles.header}>
        <Pressable onPress={() => router.back()} hitSlop={8}>
          <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
        </Pressable>
        <Text style={[styles.title, { color: tokens.text }]}>Offline queue</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          {pendingCount === 0
            ? "Nothing waiting — every change reached the server."
            : `${pendingCount} change${pendingCount === 1 ? "" : "s"} waiting or retrying.`}
        </Text>
      </View>

      <Button
        label="Sync now"
        testID="queue-sync-now"
        fullWidth
        disabled={pendingCount === 0}
        onPress={() => void engine?.catchUp()}
      />

      {rejected.length > 0 ? <SectionHeader title="Needs review" /> : null}
      {rejected.map((op) => (
        <QueueOpCard key={op.id} op={op} />
      ))}

      <SectionHeader title="Pending" />
      {pending.length === 0 ? <EmptyState message="No queued changes." /> : null}
      {pending.map((op) => (
        <QueueOpCard key={op.id} op={op} />
      ))}
    </Screen>
  );
}

/** The op's wire path with the version noise removed: UUIDs shorten to
 * eight characters and the /api/v1 prefix drops, so the line stays readable
 * while the full path rides the audit trail. */
function shortPath(op: QueuedOp): string {
  const path = op.path
    .replace("/api/v1", "")
    .replace(
      /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g,
      (match) => match.slice(0, 8),
    );
  return `${op.method} ${path}`;
}

function QueueOpCard({ op }: { op: QueuedOp }) {
  const { tokens } = useTheme();
  const engine = useEngine();
  const [expanded, setExpanded] = useState(false);
  const rejected = op.state === "rejected";

  return (
    <Card testID={`queue-op-${op.id}`} style={styles.card}>
      <View style={styles.row}>
        <Text style={[styles.label, { color: tokens.text }]}>{op.label}</Text>
        <Pill
          pair={rejected ? tokens.danger : op.state === "pending" ? tokens.warning : accentPair(tokens)}
          label={op.state.toUpperCase()}
        />
      </View>
      <Mono muted>{shortPath(op)}</Mono>
      {op.failure !== undefined ? (
        <Text style={[styles.failure, { color: tokens.danger.text }]}>{op.failure.message}</Text>
      ) : null}
      <Pressable onPress={() => setExpanded((value) => !value)} hitSlop={8}>
        <Text style={[styles.expand, { color: tokens.primary }]}>
          {expanded ? "Hide trail" : `Trail (${op.audit.length})`}
        </Text>
      </Pressable>
      {expanded
        ? op.audit.map((entry, index) => (
            <Text key={`${entry.at}-${index}`} style={[styles.audit, { color: tokens.textMuted }]}>
              {new Date(entry.at).toLocaleTimeString()} · {entry.kind}
              {entry.detail !== undefined ? ` — ${entry.detail}` : ""}
            </Text>
          ))
        : null}
      {rejected ? (
        <View style={styles.actions}>
          <Button
            label="Retry"
            testID={`queue-retry-${op.id}`}
            onPress={() => {
              void engine?.queue.retryRejected(op.id).then(() => engine.catchUp());
            }}
          />
          <Button
            label="Discard"
            variant="secondary"
            testID={`queue-discard-${op.id}`}
            onPress={() => void engine?.queue.drop(op.id)}
          />
        </View>
      ) : null}
    </Card>
  );
}

const styles = StyleSheet.create({
  header: {
    paddingTop: 8,
    gap: 4,
  },
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  card: {
    gap: 6,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  label: {
    flex: 1,
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  failure: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  expand: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  audit: {
    fontFamily: type.mono,
    fontSize: 11,
    lineHeight: 16,
  },
  actions: {
    flexDirection: "row",
    gap: 8,
    paddingTop: 4,
  },
});
