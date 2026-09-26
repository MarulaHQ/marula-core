// Photo attachments on ticket notes — the upload-first composer flow.
//
// Purpose: pick a photo (library or camera), upload the raw bytes against
// the ticket, and hand the returned attachment id to the comment create —
// the same order the workbench composer uses. The upload needs the wire, so
// an offline attempt surfaces the honest offline message instead of
// pretending to queue bytes; the note itself still queues normally.
//
// Non-purpose: no binary queueing (the write queue carries JSON ops), no
// image editing, no document attachments.

import * as ImagePicker from "expo-image-picker";
import { Image } from "react-native";

import { DEV_TOOLS_ENABLED } from "@/config";

export interface PickedPhoto {
  uri: string;
  name: string;
  mimeType: string;
  sizeBytes: number;
}

export interface StagedAttachment {
  id: string;
  filename: string;
  contentType: string;
  sizeBytes: number;
}

/** The dev-menu sample, bundled so the attach path is exercisable without
 * the system picker (Maestro flows and capture runs). */
export const SAMPLE_PHOTO = require("../../assets/sample-site-photo.png") as number;

export function samplePhoto(): PickedPhoto {
  const asset = Image.resolveAssetSource(SAMPLE_PHOTO);
  return {
    uri: asset.uri,
    name: "site-panel.png",
    mimeType: "image/png",
    sizeBytes: 2099,
  };
}

export class AttachmentUploadError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "AttachmentUploadError";
    this.status = status;
  }
}

/** Opens the photo library. Returns null when the technician cancels. */
export async function pickPhoto(): Promise<PickedPhoto | null> {
  const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (!permission.granted) {
    throw new AttachmentUploadError(0, "Photo access is off — allow it in Settings to attach pictures.");
  }
  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ["images"],
    quality: 0.7,
    allowsMultipleSelection: false,
  });
  if (result.canceled || result.assets.length === 0) return null;
  const asset = result.assets[0]!;
  const fallbackName = asset.uri.split("/").pop() ?? `photo-${Date.now()}.jpg`;
  return {
    uri: asset.uri,
    name: asset.fileName ?? fallbackName,
    mimeType: asset.mimeType ?? mimeFromName(asset.fileName ?? fallbackName),
    sizeBytes: asset.fileSize ?? 0,
  };
}

/** Uploads one picked file against the ticket. The caller's access token
 * rides the Authorization header; a 401 means the session needs a refresh
 * before the note can carry the photo. */
export async function uploadTicketAttachment(options: {
  apiUrl: string;
  accessToken: string;
  ticketId: string;
  file: PickedPhoto;
}): Promise<StagedAttachment> {
  const { apiUrl, accessToken, ticketId, file } = options;
  const local = await fetch(file.uri);
  const blob = await local.blob();
  const response = await fetch(
    `${apiUrl}/api/v1/tickets/${encodeURIComponent(ticketId)}/attachments`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": file.mimeType,
        "x-filename": file.name,
      },
      body: blob,
    },
  );
  if (!response.ok) {
    const detail = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new AttachmentUploadError(
      response.status,
      detail?.error?.message ?? `The photo upload was rejected (${response.status}).`,
    );
  }
  const attachment = (await response.json()) as {
    id: string;
    filename: string;
    content_type: string;
    size_bytes: number;
  };
  return {
    id: attachment.id,
    filename: attachment.filename,
    contentType: attachment.content_type,
    sizeBytes: attachment.size_bytes,
  };
}

export function attachmentUrl(apiUrl: string, attachmentId: string): string {
  return `${apiUrl}/api/v1/attachments/${encodeURIComponent(attachmentId)}/download`;
}

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const MIME_BY_EXTENSION: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
};

function mimeFromName(name: string): string {
  const extension = name.toLowerCase().split(".").pop() ?? "";
  return MIME_BY_EXTENSION[extension] ?? "image/jpeg";
}

/** Dev-menu attach injector: the sample rides the same upload path the
 * picker feeds, so flow tests and captures stay deterministic. */
export const ATTACH_INJECTOR_ENABLED = DEV_TOOLS_ENABLED;
