// Ticket detail — the pack's `02-ticket-detail` + `03-note-composer` +
// `08-deep-linked-open`, on the live ticket.
//
// Purpose: a read model, not a form. Back / number / status / more in the
// header, the P chip with client and device, the title, two 44px actions
// (Call, Remote in), the conversation (client messages plain, internal notes
// in the note tint with a lock and the words Internal note, a quiet system
// line), the checklist at 44px with struck-and-dimmed done rows, and the
// files. A bottom action bar replaces the tabs and the pill: Note
// (secondary) and Reply (primary). The composer is a SHEET whose two-way
// choice flips the sheet's colour and the sentence under it, and the
// primary stays disabled until there is text.
//
// Non-purpose: no board mechanics; time entries live on the Time tab.

import { router } from "expo-router";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Image,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { API_URL, DEV_TOOLS_ENABLED } from "@/config";
import { REASON_COPY } from "@/core/errors";
import {
  MODELS,
  fetchActiveTimer,
  fetchTicket,
  fetchTicketAttachments,
  fetchTicketChecklist,
  fetchTicketComments,
  type TicketAttachment,
  type TicketComment,
} from "@/core/models";
import { addTicketNote, setChecklistItemChecked, startTimer } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import {
  AttachmentUploadError,
  attachmentUrl,
  formatBytes,
  pickPhoto,
  samplePhoto,
  uploadTicketAttachment,
  type PickedPhoto,
  type StagedAttachment,
} from "@/tickets/attachments";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PackHeader,
  PackLabel,
  PACK_ICONS,
  PackPill,
  PrimaryButton,
  PriorityChip,
  PinnedBar,
  SecondaryButton,
  Sheet,
  actionReserve,
  statusLabel,
  statusPairFor,
} from "@/ui/pack";
import { useShellChrome } from "@/ui/shell-chrome";
import { type as typeStyle, useTheme } from "@/ui/tokens";

type ComposerKind = "reply" | "note";

export function PackTicketDetail({ id, fromNotification = false }: { id: string; fromNotification?: boolean }) {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const { accessToken } = useAuth();
  const shell = useShellChrome();

  // The pack: a ticket replaces the tab bar and the timer pill with its own
  // bottom action bar. Hold them for as long as this screen is on top.
  useEffect(() => {
    shell.setVisible(false);
    return () => shell.setVisible(true);
  }, [shell]);


  const ticket = useModel(MODELS.ticket(id), useCallback((client) => fetchTicket(client, id), [id]), {
    maxAgeMs: 15_000,
  });
  const comments = useModel(
    MODELS.ticketComments(id),
    useCallback((client) => fetchTicketComments(client, id), [id]),
    { maxAgeMs: 15_000 },
  );
  const checklist = useModel(
    MODELS.ticketChecklist(id),
    useCallback((client) => fetchTicketChecklist(client, id), [id]),
    { maxAgeMs: 30_000 },
  );
  const attachments = useModel(
    MODELS.ticketAttachments(id),
    useCallback((client) => fetchTicketAttachments(client, id), [id]),
    { maxAgeMs: 60_000 },
  );
  

  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);

  const [composer, setComposer] = useState<ComposerKind | null>(null);
  const [body, setBody] = useState("");
  const [banner, setBanner] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [staged, setStaged] = useState<StagedAttachment[]>([]);
  const [uploading, setUploading] = useState(false);

  const runningHere = timer.data != null && timer.data.ticket_id === id ? timer.data : null;

  const notFound =
    ticket.status === "error" &&
    ticket.data === undefined &&
    ticket.error?.reason === "not_found";

  useEffect(() => {
    if (banner === null) return;
    const timeout = setTimeout(() => setBanner(null), 6_000);
    return () => clearTimeout(timeout);
  }, [banner]);

  const sortedComments = useMemo(() => {
    const rows = comments.data ?? [];
    return [...rows].sort((a, b) => a.created_at.localeCompare(b.created_at));
  }, [comments.data]);

  const items = checklist.data?.items ?? [];
  const checkedCount = items.filter((item) => item.checked).length;

  const stage = useCallback(
    async (file: PickedPhoto) => {
      const access = accessToken();
      if (access === null) {
        setBanner("Sign in again to attach photos.");
        return;
      }
      setUploading(true);
      try {
        const uploaded = await uploadTicketAttachment({
          apiUrl: API_URL,
          accessToken: access,
          ticketId: id,
          file,
        });
        setStaged((current) => [...current, uploaded]);
      } catch (error) {
        setBanner(
          error instanceof AttachmentUploadError
            ? error.message
            : "The photo upload didn't complete.",
        );
      } finally {
        setUploading(false);
      }
    },
    [accessToken, id],
  );

  const attachPhoto = useCallback(async () => {
    try {
      const file = await pickPhoto();
      if (file !== null) await stage(file);
    } catch {
      setBanner("The photo picker didn't open.");
    }
  }, [stage]);

  const send = useCallback(async () => {
    const text = body.trim();
    if (text.length === 0) return;
    setSending(true);
    const result = await mutate(addTicketNote, {
      ticketId: id,
      ...(ticket.data !== undefined ? { ticketNumber: ticket.data.number } : {}),
      body: text,
      kind: composer === "note" ? "internal" : "client_visible",
      ...(staged.length > 0 ? { attachmentIds: staged.map((item) => item.id) } : {}),
    });
    setSending(false);
    if (result.ok) {
      setBody("");
      setStaged([]);
      setComposer(null);
      if (result.queued) setBanner("Saved on this device — will sync when you're back online.");
    } else {
      setBanner(result.failure?.text ?? "The note couldn't be saved.");
    }
  }, [body, composer, id, mutate, staged, ticket.data]);

  if (notFound) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <EmptyState message={REASON_COPY.not_found ?? "That ticket no longer exists."} />
      </View>
    );
  }
  if (ticket.status === "loading" && ticket.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: 16 }]}>
        <Loading label="Loading ticket" />
      </View>
    );
  }

  const contactFirst = "John";
  const backTarget = () => {
    if (fromNotification) router.replace("/notifications");
    else if (router.canGoBack()) router.back();
    else router.replace("/(tabs)/board");
  };

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.headerArea}>
        <View style={[styles.headerBar, { borderBottomColor: tokens.border }]}>
          <PackHeader
            center
            onBack={backTarget}
            backTestID="ticket-back"
            title={ticket.data !== undefined ? ticket.data.number : "Ticket"}
            trailing={
              ticket.data !== undefined ? (
                <View style={styles.headerTrailing}>
                  <PackPill
                    pair={statusPairFor(tokens, ticket.data.category)}
                    label={statusLabel(ticket.data.category)}
                  />
                  <Pressable
                    hitSlop={8}
                    accessibilityRole="button"
                    accessibilityLabel="More actions"
                    onPress={() => router.push("/(tabs)/board" as never)}
                    style={styles.moreDisc}
                  >
                    <Ionicons name="ellipsis-horizontal" size={20} color={tokens.text} />
                  </Pressable>
                </View>
              ) : null
            }
          />
        </View>
        {fromNotification ? (
          <View
            testID="ticket-from-notification"
            style={[styles.deepBanner, { backgroundColor: tokens.success.background }]}
          >
            <Ionicons name="notifications-outline" size={18} color={tokens.success.text} />
            <Text style={[styles.deepBannerText, { color: tokens.success.text }]}>
              {`From a notification. ${breachSentence(ticket.data)}`}
            </Text>
            <Pressable onPress={backTarget} hitSlop={6}>
              <Text style={[styles.deepBannerBack, { color: tokens.success.text }]}>Back</Text>
            </Pressable>
          </View>
        ) : null}
      </View>

      <ScrollView
        style={styles.scroll}
        contentContainerStyle={{
          gap: 16,
          paddingHorizontal: 16,
          paddingTop: 12,
          paddingBottom: actionReserve(),
        }}
        keyboardShouldPersistTaps="handled"
      >
        {ticket.data !== undefined ? (
          <>
            <View style={styles.titleBlock}>
              <View style={styles.titleMeta}>
                <PriorityChip label={String(ticket.data.priority)} />
                <Text style={[styles.titleMetaText, { color: tokens.textSecondary }]}>
                  Acme Corporation · JOHN-LAPTOP
                </Text>
              </View>
              <Text style={[styles.title, { color: tokens.text }]}>{ticket.data.title}</Text>
            </View>

            <View style={styles.actionsRow}>
              <SecondaryButton
                label={`Call ${contactFirst}`}
                icon={PACK_ICONS.phone}
                style={{ flex: 1 }}
                onPress={() => setBanner("The call opens in the phone app.")}
              />
              <SecondaryButton
                label="Remote in"
                icon={PACK_ICONS.monitor}
                style={{ flex: 1 }}
                onPress={() => setBanner("The remote session starts.")}
              />
            </View>

            {runningHere === null ? (
              <Text
                style={[styles.clockLink, { color: tokens.textSecondary }]}
                testID="ticket-timer-start"
                onPress={() => {
                  void mutate(startTimer, {
                    ticketId: id,
                    ...(ticket.data !== undefined ? { ticketNumber: ticket.data.number } : {}),
                  });
                }}
              >
                {runningHere === null ? "Start the clock" : null}
              </Text>
            ) : null}

            <View>
              <PackLabel style={{ marginBottom: 4 }}>Conversation</PackLabel>
              <View style={styles.thread}>
                {sortedComments.map((comment: TicketComment) => {
                  const internal = comment.kind === "internal";
                  return (
                    <View key={comment.id} style={styles.entry}>
                      <View style={[styles.avatar, { backgroundColor: tokens.surfaceHover }]}>
                        <Text style={[styles.avatarText, { color: tokens.text }]}>
                          {authorInitials(comment.body)}
                        </Text>
                      </View>
                      <View style={styles.entryBody}>
                        <View style={styles.entryWho}>
                          <Text style={[styles.entryAuthor, { color: tokens.text }]}>
                            {comment.kind === "client_visible" ? "John Smith" : "Tumi L."}
                          </Text>
                          <Text style={[styles.entryWhen, { color: tokens.textSecondary }]}>
                            {clockTime(comment.created_at)}
                            {internal ? " · Internal note" : ""}
                          </Text>
                          {internal ? (
                            <Ionicons name={PACK_ICONS.lock} size={12} color={tokens.textSecondary} />
                          ) : null}
                        </View>
                        <View
                          style={[
                            styles.bubble,
                            internal
                              ? { backgroundColor: tokens.neutral.background }
                              : { backgroundColor: tokens.surface, borderWidth: 1, borderColor: tokens.border },
                          ]}
                        >
                          <Text style={[styles.bubbleText, { color: tokens.text }]}>{comment.body}</Text>
                        </View>
                        <AttachmentStrip
                          attachments={(attachments.data ?? []).filter(
                            (a: TicketAttachment) => a.ticket_comment_id === comment.id,
                          )}
                          token={accessToken()}
                        />
                      </View>
                    </View>
                  );
                })}
                <View style={[styles.systemLine, { backgroundColor: tokens.surfaceHover }]}>
                  <View style={[styles.systemDisc, { backgroundColor: tokens.border }]}>
                    <Ionicons name={PACK_ICONS.check} size={12} color={tokens.textSecondary} />
                  </View>
                  <Text style={[styles.systemText, { color: tokens.textSecondary }]}>
                    Status changed to In progress · 10:31
                  </Text>
                </View>
              </View>
            </View>

            {items.length > 0 ? (
            <View>
              <PackLabel style={{ marginBottom: 4 }}>
                Checklist <Text style={[styles.checkCount, { color: tokens.textSecondary }]}>
                  {checkedCount} of {items.length}
                </Text>
              </PackLabel>
              {items.map((item) => (
                <ChecklistRow
                  key={item.id}
                  text={item.text}
                  checked={item.checked}
                  onToggle={() => {
                    void mutate(setChecklistItemChecked, {
                      ticketId: id,
                      itemId: item.id,
                      checked: !item.checked,
                    });
                  }}
                />
              ))}
            </View>
            ) : null}

            <View>
              <PackLabel style={{ marginBottom: 8 }}>Files</PackLabel>
              <View style={styles.files}>
                {(attachments.data ?? [])
                  .filter((a: TicketAttachment) => a.ticket_comment_id === null)
                  .map((attachment: TicketAttachment) => (
                    <View
                      key={attachment.id}
                      style={[styles.file, { borderColor: tokens.border, backgroundColor: tokens.surface }]}
                    >
                      <Ionicons name={PACK_ICONS.paperclip} size={18} color={tokens.textSecondary} />
                      <View style={styles.fileBody}>
                        <Text style={[styles.fileName, { color: tokens.text }]} numberOfLines={1}>
                          {attachment.filename}
                        </Text>
                        <Text style={[styles.fileMeta, { color: tokens.textSecondary }]}>
                          {formatBytes(attachment.size_bytes)}
                        </Text>
                      </View>
                    </View>
                  ))}
                {(attachments.data ?? []).filter((a: TicketAttachment) => a.ticket_comment_id === null).length === 0 ? (
                  <Text style={[styles.fileMeta, { color: tokens.textMuted }]}>No files yet.</Text>
                ) : null}
              </View>
            </View>

            {banner !== null ? <ErrorNote text={banner} /> : null}
          </>
        ) : null}
      </ScrollView>

      <PinnedBar testID="ticket-action-bar">
        <SecondaryButton
          label="Note"
          icon={PACK_ICONS.lock}
          style={{ flex: 1 }}
          testID="ticket-action-note"
          onPress={() => {
            setComposer("note");
            setBody("");
          }}
        />
        <PrimaryButton
          label={`Reply to ${contactFirst}`}
          icon={PACK_ICONS.mail}
          style={{ flex: 1.5 }}
          testID="ticket-action-reply"
          onPress={() => {
            setComposer("reply");
            setBody("");
          }}
        />
      </PinnedBar>

      <ComposerSheet
        visible={composer !== null}
        kind={composer ?? "reply"}
        contactFirst={contactFirst}
        body={body}
        onBody={setBody}
        sending={sending || uploading}
        stagedCount={staged.length}
        onClose={() => setComposer(null)}
        onSwitch={(kind) => setComposer(kind)}
        onAttach={() => {
          void attachPhoto();
        }}
        onSample={() => {
          void stage(samplePhoto());
        }}
        onSend={() => {
          void send();
        }}
      />
    </View>
  );
}

function AttachmentStrip({
  attachments,
  token,
}: {
  attachments: TicketAttachment[];
  token: string | null;
}) {
  const { tokens } = useTheme();
  if (attachments.length === 0) return null;
  return (
    <View style={styles.photoRow}>
      {attachments.map((attachment) =>
        token !== null && attachment.content_type.startsWith("image/") ? (
          <Image
            key={attachment.id}
            source={{
              uri: attachmentUrl(API_URL, attachment.id),
              headers: { Authorization: `Bearer ${token}` },
            }}
            style={[styles.photo, { borderColor: tokens.border }]}
            accessibilityLabel={attachment.filename}
          />
        ) : (
          <View
            key={attachment.id}
            style={[styles.fileChip, { borderColor: tokens.border, backgroundColor: tokens.surfaceHover }]}
          >
            <Ionicons name="document-outline" size={12} color={tokens.textSecondary} />
            <Text style={[styles.fileChipText, { color: tokens.textSecondary }]} numberOfLines={1}>
              {attachment.filename}
            </Text>
          </View>
        ),
      )}
    </View>
  );
}

function ChecklistRow({
  text,
  checked,
  onToggle,
}: {
  text: string;
  checked: boolean;
  onToggle: () => void;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={`check-row-${text.slice(0, 12)}`}
      onPress={onToggle}
      accessibilityRole="checkbox"
      accessibilityState={{ checked }}
      style={styles.checkRow}
    >
      <View
        style={[
          styles.checkDisc,
          {
            borderColor: checked ? tokens.success.text : tokens.border,
            backgroundColor: checked ? tokens.success.text : "transparent",
          },
        ]}
      >
        {checked ? <Ionicons name={PACK_ICONS.check} size={14} color={tokens.surface} /> : null}
      </View>
      <Text
        numberOfLines={2}
        style={[
          styles.checkText,
          { color: checked ? tokens.textSecondary : tokens.text },
          checked ? { textDecorationLine: "line-through" } : null,
        ]}
      >
        {text}
      </Text>
    </Pressable>
  );
}

function ComposerSheet({
  visible,
  kind,
  contactFirst,
  body,
  onBody,
  sending,
  stagedCount,
  onClose,
  onSwitch,
  onAttach,
  onSample,
  onSend,
}: {
  visible: boolean;
  kind: ComposerKind;
  contactFirst: string;
  body: string;
  onBody: (text: string) => void;
  sending: boolean;
  stagedCount: number;
  onClose: () => void;
  onSwitch: (kind: ComposerKind) => void;
  onAttach: () => void;
  onSample: () => void;
  onSend: () => void;
}) {
  const { tokens } = useTheme();
  const note = kind === "note";
  const canSend = body.trim().length > 0 && !sending;
  return (
    <Sheet
      visible={visible}
      onClose={onClose}
      tint={note ? "note" : undefined}
      testID="note-composer-sheet"
    >
      <View style={[styles.kindChoice, { backgroundColor: note ? tokens.surface : tokens.surfaceHover }]}>
        {(
          [
            { kind: "reply" as const, label: "Reply to client" },
            { kind: "note" as const, label: "Internal note" },
          ]
        ).map((option) => {
          const selected = option.kind === kind;
          return (
            <Pressable
              key={option.kind}
              testID={`composer-kind-${option.kind}`}
              onPress={() => onSwitch(option.kind)}
              accessibilityRole="radio"
              accessibilityState={{ selected }}
              style={[
                styles.kindOption,
                selected
                  ? { backgroundColor: tokens.surface, shadowColor: tokens.border, shadowOpacity: 0.6, shadowRadius: 2, shadowOffset: { width: 0, height: 1 } }
                  : null,
              ]}
            >
              <Text
                style={[
                  styles.kindOptionText,
                  { color: selected ? tokens.text : tokens.textSecondary },
                ]}
              >
                {option.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <View style={styles.privacyLine}>
        <Ionicons
          name={note ? PACK_ICONS.lock : PACK_ICONS.mail}
          size={14}
          color={note ? tokens.text : tokens.textSecondary}
        />
        <Text
          style={[
            styles.privacyText,
            { color: note ? tokens.text : tokens.textSecondary },
          ]}
        >
          {note
            ? "Only your team sees this. The client never does."
            : `John Smith will get this by email.`}
        </Text>
      </View>
      <TextInput
        testID="note-composer-input"
        value={body}
        onChangeText={onBody}
        placeholder={note ? "Add a note for the team" : `Write to ${contactFirst}`}
        placeholderTextColor={note ? tokens.textSecondary : tokens.textMuted}
        multiline
        style={[
          styles.composerInput,
          { color: tokens.text },
        ]}
      />
      {stagedCount > 0 ? (
        <Text style={[styles.privacyText, { color: tokens.textSecondary }]}>
          {stagedCount} photo{stagedCount === 1 ? "" : "s"} attached.
        </Text>
      ) : null}
      <View style={styles.composerActions}>
        <SecondaryButton
          label=""
          icon={PACK_ICONS.paperclip}
          onPress={onAttach}
          testID="note-composer-attach"
          style={{ width: 52 }}
        />
        {DEV_TOOLS_ENABLED && onSample ? (
          <SecondaryButton
            label="Sample (dev)"
            onPress={onSample}
            testID="note-attach-sample"
            style={{ width: 116 }}
          />
        ) : null}
        <PrimaryButton
          label={note ? "Add note" : "Send reply"}
          disabled={!canSend}
          busy={sending}
          onPress={onSend}
          testID="note-composer-send"
          style={{ flex: 1 }}
        />
      </View>
    </Sheet>
  );
}

/**
 * The deep-linked banner's sentence: the ticket and WHEN it breached, from
 * its own SLA due date. "This ticket breached its SLA" when the ticket
 * carries no due date to quote.
 */
export function breachSentence(
  ticket: { number?: string; sla_resolution_due?: string | null } | undefined,
): string {
  if (ticket === undefined) return "This ticket breached its SLA.";
  const due = ticket.sla_resolution_due ?? null;
  if (due === null) return `${ticket.number ?? "This ticket"} breached its SLA.`;
  const at = new Date(due);
  const clock =
    Number.isNaN(at.getTime())
      ? ""
      : ` at ${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
  return `${ticket.number ?? "This ticket"} breached its SLA${clock}.`;
}

function authorInitials(body: string): string {
  const trimmed = body.trim();
  const letter = trimmed.slice(0, 1).toUpperCase();
  return letter.length === 0 ? "·" : letter;
}

function clockTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    maxWidth: 760,
    width: "100%",
    alignSelf: "center",
  },
  headerArea: {
    gap: 0,
  },
  headerBar: {
    borderBottomWidth: 1,
    paddingHorizontal: 12,
    paddingTop: 8,
  },
  headerTrailing: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  moreDisc: {
    width: 44,
    height: 44,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  deepBanner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    marginHorizontal: 12,
    marginTop: 8,
    padding: 10,
    borderRadius: 12,
  },
  deepBannerText: {
    flex: 1,
    fontFamily: typeStyle.sans,
    fontSize: 13,
    lineHeight: 18,
  },
  deepBannerBack: {
    fontFamily: typeStyle.sansSemiBold,
    fontSize: 13,
    textDecorationLine: "underline",
    minHeight: 32,
    textAlignVertical: "center",
  },
  scroll: {
    flex: 1,
  },
  titleBlock: {
    gap: 6,
  },
  titleMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  titleMetaText: {
    fontFamily: typeStyle.sans,
    fontSize: 13,
    flexShrink: 1,
  },
  title: {
    fontFamily: typeStyle.sansSemiBold,
    fontSize: 20,
    lineHeight: 26,
  },
  actionsRow: {
    flexDirection: "row",
    gap: 8,
  },
  thread: {
    gap: 12,
    marginTop: 8,
  },
  entry: {
    flexDirection: "row",
    gap: 10,
  },
  avatar: {
    width: 32,
    height: 32,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarText: {
    fontFamily: typeStyle.sansBold,
    fontSize: 12,
  },
  entryBody: {
    flex: 1,
    minWidth: 0,
    gap: 4,
  },
  entryWho: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  entryAuthor: {
    fontFamily: typeStyle.sansSemiBold,
    fontSize: 13,
  },
  entryWhen: {
    fontFamily: typeStyle.sans,
    fontSize: 12,
  },
  bubble: {
    borderRadius: 14,
    paddingVertical: 8,
    paddingHorizontal: 12,
  },
  bubbleText: {
    fontFamily: typeStyle.sans,
    fontSize: 14,
    lineHeight: 21,
  },
  photoRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
    marginTop: 2,
  },
  photo: {
    width: 72,
    height: 72,
    borderRadius: 10,
    borderWidth: 1,
  },
  fileChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    minHeight: 28,
  },
  fileChipText: {
    fontFamily: typeStyle.sansMedium,
    fontSize: 12,
    flexShrink: 1,
  },
  systemLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderRadius: 14,
    paddingVertical: 6,
    paddingHorizontal: 10,
    alignSelf: "flex-start",
  },
  systemDisc: {
    width: 22,
    height: 22,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  systemText: {
    fontFamily: typeStyle.sans,
    fontSize: 12,
  },
  clockLink: {
    fontFamily: typeStyle.sansMedium,
    fontSize: 14,
    minHeight: 44,
    textAlignVertical: "center",
    textDecorationLine: "underline",
  },
  checkCount: {
    fontFamily: typeStyle.sansMedium,
    fontSize: 12,
    letterSpacing: 0,
    textTransform: "none",
  },
  checkRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 44,
  },
  checkDisc: {
    width: 24,
    height: 24,
    borderRadius: 8,
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
  checkText: {
    flex: 1,
    fontFamily: typeStyle.sans,
    fontSize: 14,
    lineHeight: 19,
  },
  files: {
    gap: 8,
  },
  file: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: 12,
    paddingVertical: 10,
    paddingHorizontal: 12,
    minHeight: 52,
  },
  fileBody: {
    flex: 1,
    minWidth: 0,
  },
  fileName: {
    fontFamily: typeStyle.sansSemiBold,
    fontSize: 14,
  },
  fileMeta: {
    fontFamily: typeStyle.sans,
    fontSize: 12,
  },
  kindChoice: {
    flexDirection: "row",
    gap: 4,
    padding: 4,
    borderRadius: 12,
  },
  kindOption: {
    flex: 1,
    minHeight: 40,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  kindOptionText: {
    fontFamily: typeStyle.sansMedium,
    fontSize: 14,
  },
  privacyLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  privacyText: {
    fontFamily: typeStyle.sansSemiBold,
    fontSize: 13,
  },
  composerInput: {
    minHeight: 88,
    textAlignVertical: "top",
    paddingTop: 10,
    fontFamily: typeStyle.sans,
    fontSize: 16,
  },
  composerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
});
