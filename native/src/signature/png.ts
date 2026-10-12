// A bilevel PNG encoder — pure TypeScript, zero dependencies.
//
// Purpose: React Native has no canvas and no `toDataURL`, and the payload the
// content-addressed attachment store accepts is a real PNG (the server sniffs
// magic bytes and rejects anything else). So the mark has to be encoded here.
// A two-entry palette with one transparent index is exactly the shape of a
// signature: ink and nothing, which keeps the output small enough to queue
// offline without second thoughts.
//
// Non-purpose: no compression beyond zlib "stored" blocks. A stored stream is
// larger than a deflated one and that is the right trade here: it is ~40 lines
// of well-understood arithmetic instead of a dependency, and a signature
// rasterises to tens of kilobytes, not megabytes. No colour management, no
// gamma, no interlacing — a signature needs none of them.
//
// The output is verification-checked by `native/tests/signature-png.test.ts`,
// including a decode of every chunk round-trip, so a wrong header can't ship.

/** One pixel per byte, value 0 (transparent) or 1 (ink). */
export interface BilevelImage {
  width: number;
  height: number;
  /** Two palette entries as RGB. Entry 0 is transparent. */
  palette: readonly [readonly [number, number, number], readonly [number, number, number]];
  pixels: Uint8Array;
}

const PNG_SIGNATURE = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (let i = 0; i < bytes.length; i += 1) {
    crc ^= bytes[i]!;
    for (let bit = 0; bit < 8; bit += 1) {
      const mask = -(crc & 1);
      crc = (crc >>> 1) ^ (0xedb88320 & mask);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** A length-prefixed, CRC'd chunk in the PNG wire order. */
class ChunkWriter {
  private readonly out: number[] = [];

  constructor(private readonly type: string) {}

  u8(value: number): void {
    this.out.push(value & 0xff);
  }

  u32(value: number): void {
    this.out.push((value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff);
  }

  bytes(values: readonly number[] | Uint8Array): void {
    for (const v of values) this.out.push(v & 0xff);
  }

  build(): Uint8Array {
    const typeBytes = [...this.type].map((c) => c.charCodeAt(0));
    const data = this.out;
    const length = data.length;
    const crcInput = new Uint8Array([...typeBytes, ...data]);
    const crc = crc32(crcInput);
    return new Uint8Array([
      (length >>> 24) & 0xff,
      (length >>> 16) & 0xff,
      (length >>> 8) & 0xff,
      length & 0xff,
      ...crcInput,
      (crc >>> 24) & 0xff,
      (crc >>> 16) & 0xff,
      (crc >>> 8) & 0xff,
      crc & 0xff,
    ]);
  }
}

/**
 * zlib stream with a single final, stored (uncompressed) deflate block.
 * Uncompressed is deliberate — see the module header.
 */
function zlibStored(raw: Uint8Array): Uint8Array {
  const out: number[] = [0x78, 0x01]; // CM=8, CINFO=7, no dict, FCHECK ok
  const maxBlock = 0xffff;
  for (let offset = 0; offset < raw.length; offset += maxBlock) {
    const slice = raw.subarray(offset, Math.min(offset + maxBlock, raw.length));
    const final = offset + maxBlock >= raw.length ? 1 : 0;
    out.push(final, slice.length & 0xff, (slice.length >>> 8) & 0xff);
    const nlen = ~slice.length & 0xffff;
    out.push(nlen & 0xff, (nlen >>> 8) & 0xff);
    for (const b of slice) out.push(b);
  }
  // Empty input still needs one (empty) final block.
  if (raw.length === 0) {
    out.push(1, 0, 0, 0xff, 0xff);
  }
  // Adler-32 over the raw bytes.
  let a = 1;
  let b = 0;
  for (let i = 0; i < raw.length; i += 1) {
    a = (a + raw[i]!) % 65521;
    b = (b + a) % 65521;
  }
  out.push((b >>> 8) & 0xff, b & 0xff, (a >>> 8) & 0xff, a & 0xff);
  return new Uint8Array(out);
}

/** Encodes a bilevel image as a complete PNG file. */
export function encodeBilevelPng(image: BilevelImage): Uint8Array {
  const { width, height, palette, pixels } = image;
  if (width <= 0 || height <= 0) throw new Error("a PNG needs a positive size");
  if (pixels.length !== width * height) {
    throw new Error(`pixel buffer is ${pixels.length}, expected ${width * height}`);
  }

  // IHDR: width, height, bit depth 1, colour type 3 (palette), deflate,
  // adaptive filtering, no interlace.
  const ihdr = new ChunkWriter("IHDR");
  ihdr.u32(width);
  ihdr.u32(height);
  ihdr.u8(1);
  ihdr.u8(3);
  ihdr.u8(0);
  ihdr.u8(0);
  ihdr.u8(0);

  const plte = new ChunkWriter("PLTE");
  for (const entry of palette) {
    plte.u8(entry[0]);
    plte.u8(entry[1]);
    plte.u8(entry[2]);
  }

  // tRNS: one byte per palette entry; 0 = fully transparent. Only two
  // entries here, so the array is exactly two bytes.
  const trns = new ChunkWriter("tRNS");
  trns.u8(0);
  trns.u8(255);

  // Scanlines with filter byte 0 (None), bit-packed MSB-first, 1 bit/pixel.
  const rowBytes = Math.ceil(width / 8);
  const raw = new Uint8Array((rowBytes + 1) * height);
  for (let y = 0; y < height; y += 1) {
    const rowStart = y * (rowBytes + 1);
    raw[rowStart] = 0;
    for (let x = 0; x < width; x += 1) {
      if (pixels[y * width + x] !== 0) {
        const at = rowStart + 1 + (x >>> 3);
        // Same reasoning as `b64` above: the index is in range by construction.
        raw[at] = (raw[at] ?? 0) | (0x80 >>> (x & 7));
      }
    }
  }

  const idat = new ChunkWriter("IDAT");
  idat.bytes(zlibStored(raw));

  const iend = new ChunkWriter("IEND");

  const parts = [
    PNG_SIGNATURE,
    ihdr.build(),
    plte.build(),
    trns.build(),
    idat.build(),
    iend.build(),
  ];
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let cursor = 0;
  for (const part of parts) {
    out.set(part, cursor);
    cursor += part.length;
  }
  return out;
}

/** Base64 for the JSON body — mirrors the web path's single string field. */
export function toBase64(bytes: Uint8Array): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  /** The index is masked to 0..63 and the alphabet is 64 characters, so the
   *  lookup cannot miss; `noUncheckedIndexedAccess` cannot prove that, and a
   *  non-null assertion would hide a real hole if it ever were wrong. */
  const b64 = (index: number): string => alphabet[index & 63] ?? "";
  let out = "";
  let i = 0;
  for (; i + 2 < bytes.length; i += 3) {
    const n = (bytes[i]! << 16) | (bytes[i + 1]! << 8) | bytes[i + 2]!;
    out += b64(n >>> 18) + b64(n >>> 12) + b64(n >>> 6) + b64(n);
  }
  const remaining = bytes.length - i;
  if (remaining === 1) {
    const n = bytes[i]! << 16;
    out += `${b64(n >>> 18)}${b64(n >>> 12)}==`;
  } else if (remaining === 2) {
    const n = (bytes[i]! << 16) | (bytes[i + 1]! << 8);
    out += `${b64(n >>> 18)}${b64(n >>> 12)}${b64(n >>> 6)}=`;
  }
  return out;
}
