// Skeleton — the native counterpart of the web pack's flat loading blocks.
//
// Purpose: hold the shape of what is arriving, so a pending screen reads as
// loading rather than as empty, and the content does not shove the page around
// when it lands. The web kit (packages/ui/src/components/skeleton.tsx) draws
// this state as still blocks with a sentence that says what is loading, and the
// phone keeps that rule.
//
// The row height is the caller's, because the surfaces differ: a ticket row is
// 76px, a notification is 72, a checkout line is 64. Each caller passes its own
// `minHeight` and line count rather than stretching one shape over four lists —
// an approximation that is 20px off is a visible jump, which is the thing a
// skeleton exists to prevent.
//
// Non-purpose: no shimmer, no pulse. docs/theme.md §5 allows transform and
// opacity only, and the pack draws loading still; a skeleton that animates is a
// second thing competing with the entrance reveal. Not for short in-flight
// actions either — a button or one line that is being written keeps `Loading`.

import { StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";

import { shape as shapeScale, type, useTheme } from "./tokens";

/** The three shapes the pack draws: a text line, a block, an avatar. */
export type SkeletonShape = "line" | "block" | "circle";

export function Skeleton({
  shape = "line",
  width,
  height,
  style,
}: {
  shape?: SkeletonShape;
  /** Length, percentage or px. Defaults to the shape's own width. */
  width?: number | `${number}%`;
  /** Length or px. Defaults to the shape's own height. */
  height?: number | `${number}%`;
  style?: StyleProp<ViewStyle>;
}): React.JSX.Element {
  const { tokens } = useTheme();
  return (
    <View
      // The blocks carry no meaning: the group's label is what a screen
      // reader hears, and each block is skipped rather than announced.
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        styles.block,
        { backgroundColor: tokens.surfaceHover },
        shape === "line" ? styles.line : shape === "block" ? styles.square : styles.circle,
        width !== undefined ? { width } : null,
        height !== undefined ? { height } : null,
        style,
      ]}
    />
  );
}

/**
 * Blocks plus the words that say what is happening, announced once.
 *
 * A screen reader hears the sentence when the caller supplied one, otherwise
 * the label, so it reads the same copy a sighted reader sees on the page.
 */
export function SkeletonGroup({
  label,
  sentence,
  children,
  style,
  testID,
}: {
  /** Accessible name. Ignored when `sentence` is set, which is then announced. */
  label: string;
  sentence?: string;
  children?: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  testID?: string;
}): React.JSX.Element {
  const { tokens } = useTheme();
  const spoken = sentence ?? `${label}…`;
  return (
    <View
      testID={testID}
      accessible
      accessibilityRole="progressbar"
      accessibilityState={{ busy: true }}
      accessibilityLabel={spoken}
      style={[styles.group, style]}
    >
      {children}
      <Text style={[styles.sentence, { color: tokens.textSecondary }]}>{spoken}</Text>
    </View>
  );
}

/**
 * A list of rows waiting. The defaults draw the ticket row as it actually is: a
 * priority chip, a 20px title line, a 15px meta line, a 24px pill pair and the
 * chevron, with the row's own 4px gaps and 12px padding.
 *
 * That is 20 + 4 + 15 + 4 + 24 = 67 of content, plus 24 of padding and the 1px
 * rule every row carries, loaded or waiting: 92px. The loaded one-line row is
 * the same 20 (title lineHeight) + 4 + 15 (the 12px mono meta) + 4 + 24 (pack
 * pill height) + 24 + 1 = 92, with no extra margin on its pill row — so the
 * placeholder must not add one either.
 *
 * A wrapped two-line title arrives 20px taller than the placeholder. Reserving
 * the second line instead would pad every one-line row by 20px, and a one-line
 * title is the common case, so the placeholder holds the typical row and the
 * rare wrapped title moves the list by one line.
 *
 * Every other list passes its own shape. `minHeight` must be the real row's
 * minimum, `lines` the number of title lines, and the boolean toggles exist
 * because a notification has no pills and a checkout line has no chevron.
 */
export function SkeletonRows({
  label,
  sentence,
  rows = 6,
  minHeight = 76,
  lines = 1,
  meta = true,
  leading = true,
  pills = true,
  chevron = true,
}: {
  label: string;
  sentence?: string;
  rows?: number;
  /** The real row's minimum height, so the list does not move on arrival. */
  minHeight?: number;
  /** Title lines in the real row. The default matches a one-line title. */
  lines?: number;
  /** Draw the meta line under the title. */
  meta?: boolean;
  /** Draw the leading chip. Off for rows that start at the text. */
  leading?: boolean;
  /** Draw the trailing pill pair (ticket rows only). */
  pills?: boolean;
  /** Draw the trailing chevron. */
  chevron?: boolean;
}): React.JSX.Element {
  const { tokens } = useTheme();
  return (
    <SkeletonGroup label={label} sentence={sentence} testID="skeleton-rows">
      {/* A real list has no gap between rows — the rule under each row is the
          separator. The group's 10px stack is for blocks, not for a list, so
          the rows get their own zero-gap container and the pitch stays the
          row's own height. */}
      <View style={styles.rowStack}>
        {Array.from({ length: rows }, (_, index) => (
          <View
            key={index}
            style={[styles.row, { borderBottomColor: tokens.border, minHeight }]}
          >
            {leading ? (
              <Skeleton shape="block" width={28} height={22} style={styles.chip} />
            ) : null}
            <View style={styles.rowBody}>
              {Array.from({ length: lines }, (_, line) => (
                <Skeleton
                  key={`title-${line}`}
                  width={index % 3 === 1 ? "54%" : "72%"}
                  height={20}
                  style={styles.titleLine}
                />
              ))}
              {meta ? <Skeleton width={index % 2 === 0 ? "40%" : "62%"} height={15} /> : null}
              {pills ? (
                <View style={styles.pills}>
                  <Skeleton shape="block" width={64} height={24} style={styles.pill} />
                  <Skeleton shape="block" width={52} height={24} style={styles.pill} />
                </View>
              ) : null}
            </View>
            {chevron ? (
              <Skeleton shape="circle" width={18} height={18} style={styles.chevron} />
            ) : null}
          </View>
        ))}
      </View>
    </SkeletonGroup>
  );
}

/**
 * A detail screen waiting: a title block, an identity row, then the fact grid.
 *
 * Pass `facts={0}` for a record whose page opens with a header and a card
 * instead of a fact grid — an empty bordered box would be a shape the content
 * never has.
 */
export function SkeletonDetail({
  label,
  sentence,
  facts = 4,
  inset = true,
}: {
  label: string;
  sentence?: string;
  facts?: number;
  /** Adds the screen gutter. Pass false inside a container that has one. */
  inset?: boolean;
}): React.JSX.Element {
  const { tokens } = useTheme();
  return (
    <SkeletonGroup
      label={label}
      sentence={sentence}
      testID="skeleton-detail"
      style={[styles.detail, inset ? styles.detailInset : null]}
    >
      <View style={styles.detailHead}>
        <Skeleton width="62%" height={22} />
        <Skeleton width="32%" height={13} />
      </View>
      <View style={styles.identity}>
        <Skeleton shape="block" width={56} height={56} style={styles.tile} />
        <View style={styles.identityPills}>
          <Skeleton shape="block" width={78} height={24} style={styles.pill} />
          <Skeleton shape="block" width={62} height={24} style={styles.pill} />
        </View>
      </View>
      {facts > 0 ? (
        <View style={[styles.facts, { borderColor: tokens.border }]}>
          {Array.from({ length: facts }, (_, index) => (
            <View
              key={index}
              style={[
                styles.factRow,
                index < facts - 1 ? [styles.factRule, { borderBottomColor: tokens.border }] : null,
              ]}
            >
              <Skeleton width={72} height={13} />
              <Skeleton width={index % 2 === 0 ? 96 : 64} height={13} />
            </View>
          ))}
        </View>
      ) : null}
    </SkeletonGroup>
  );
}

/**
 * A card waiting: for a widget or a list of items.
 *
 * `bare` drops the card's own border, radius and padding, for the case where
 * the caller already sits inside a `Card` — two nested borders is a shape the
 * loaded content never has.
 */
export function SkeletonCard({
  label,
  sentence,
  lines = 3,
  bare = false,
  style,
}: {
  label: string;
  sentence?: string;
  lines?: number;
  /** Render only the inner shape, for use inside an existing Card. */
  bare?: boolean;
  style?: StyleProp<ViewStyle>;
}): React.JSX.Element {
  const { tokens } = useTheme();
  return (
    <SkeletonGroup label={label} sentence={sentence} style={style}>
      <View
        style={
          bare
            ? styles.cardBare
            : [styles.card, { borderColor: tokens.border, backgroundColor: tokens.surface }]
        }
      >
        <Skeleton width="46%" height={16} />
        {Array.from({ length: lines }, (_, index) => (
          <View key={index} style={styles.cardRow}>
            <Skeleton shape="circle" width={20} height={20} />
            <View style={styles.cardRowBody}>
              <Skeleton width={index % 2 === 0 ? "68%" : "48%"} height={13} />
              <Skeleton width="34%" height={11} />
            </View>
          </View>
        ))}
      </View>
    </SkeletonGroup>
  );
}

const styles = StyleSheet.create({
  block: {
    borderRadius: shapeScale.radiusControl / 2,
  },
  line: {
    height: 12,
    borderRadius: 6,
    width: "100%",
  },
  square: {
    height: 22,
    borderRadius: 8,
    width: "100%",
  },
  circle: {
    borderRadius: shapeScale.radiusPill,
    width: shapeScale.controlHeight,
    height: shapeScale.controlHeight,
  },
  group: {
    gap: 10,
  },
  sentence: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  rowStack: {
    gap: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    paddingVertical: 12,
    paddingHorizontal: 4,
    borderBottomWidth: 1,
  },
  chip: {
    marginTop: 2,
  },
  rowBody: {
    flex: 1,
    minWidth: 0,
    gap: 4,
  },
  titleLine: {
    borderRadius: 6,
  },
  pills: {
    flexDirection: "row",
    gap: 6,
  },
  pill: {
    borderRadius: shapeScale.radiusPill,
  },
  chevron: {
    marginTop: 2,
  },
  detail: {
    paddingTop: 4,
  },
  detailInset: {
    paddingHorizontal: shapeScale.screenPadding,
  },
  detailHead: {
    gap: 8,
    paddingBottom: 4,
  },
  identity: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 8,
  },
  tile: {
    borderRadius: shapeScale.radiusCard,
  },
  identityPills: {
    flexDirection: "row",
    gap: 6,
    flexWrap: "wrap",
    flex: 1,
  },
  facts: {
    borderWidth: 1,
    borderRadius: shapeScale.radiusControl,
    overflow: "hidden",
  },
  factRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    minHeight: 44,
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  factRule: {
    borderBottomWidth: 1,
  },
  card: {
    borderWidth: 1,
    borderRadius: shapeScale.radiusCard,
    padding: 16,
    gap: 12,
  },
  cardBare: {
    gap: 12,
  },
  cardRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  cardRowBody: {
    flex: 1,
    gap: 5,
  },
});
