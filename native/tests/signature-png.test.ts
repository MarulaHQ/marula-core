// Round-trip proof for the dependency-free bilevel PNG writer.
//
// This is the load-bearing test for the native signature payload: the server
// sniffs magic bytes and accepts a real PNG or nothing, so a header bug would
// mean every signature captured on a device fails to upload. The test decodes
// every chunk back out of the produced bytes rather than trusting the writer.

import { describe, expect, it } from "vitest";

import { encodeBilevelPng, toBase64, type BilevelImage } from "../src/signature/png";

const PALETTE: BilevelImage["palette"] = [
  [0, 0, 0],
  [43, 42, 51],
];

function image(width: number, height: number, lit: (x: number, y: number) => boolean): BilevelImage {
  const pixels = new Uint8Array(width * height);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      pixels[y * width + x] = lit(x, y) ? 1 : 0;
    }
  }
  return { width, height, palette: PALETTE, pixels };
}

/** Reads big-endian u32 at `offset`. */
function u32(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset]! << 24) |
      (bytes[offset + 1]! << 16) |
      (bytes[offset + 2]! << 8) |
      bytes[offset + 3]!) >>>
    0
  );
}

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

interface Chunk {
  type: string;
  data: Uint8Array;
}

function chunks(bytes: Uint8Array): Chunk[] {
  const out: Chunk[] = [];
  let cursor = 8;
  while (cursor < bytes.length) {
    const length = u32(bytes, cursor);
    const type = String.fromCharCode(
      bytes[cursor + 4]!,
      bytes[cursor + 5]!,
      bytes[cursor + 6]!,
      bytes[cursor + 7]!,
    );
    out.push({ type, data: bytes.subarray(cursor + 8, cursor + 8 + length) });
    cursor += 12 + length;
  }
  return out;
}

describe("encodeBilevelPng", () => {
  it("starts with the PNG signature the attachment store sniffs for", () => {
    const bytes = encodeBilevelPng(image(16, 4, () => true));
    expect([...bytes.subarray(0, 8)]).toEqual(PNG_SIGNATURE);
  });

  it("emits IHDR, PLTE, tRNS, IDAT and IEND in that order", () => {
    const bytes = encodeBilevelPng(image(8, 8, (x) => x === 3));
    expect(chunks(bytes).map((c) => c.type)).toEqual(["IHDR", "PLTE", "tRNS", "IDAT", "IEND"]);
  });

  it("declares bit depth 1 with a palette colour type", () => {
    const bytes = encodeBilevelPng(image(16, 2, () => false));
    const ihdr = chunks(bytes).find((c) => c.type === "IHDR")!;
    expect(u32(ihdr.data, 0)).toBe(16);
    expect(u32(ihdr.data, 4)).toBe(2);
    expect(ihdr.data[8]).toBe(1); // bit depth
    expect(ihdr.data[9]).toBe(3); // colour type 3 = indexed
    expect(ihdr.data[10]).toBe(0); // deflate
    expect(ihdr.data[11]).toBe(0); // adaptive filtering
    expect(ihdr.data[12]).toBe(0); // no interlace
  });

  it("marks palette entry 0 transparent and entry 1 opaque", () => {
    const bytes = encodeBilevelPng(image(8, 2, () => false));
    const trns = chunks(bytes).find((c) => c.type === "tRNS")!;
    expect([...trns.data]).toEqual([0, 255]);
  });

  it("round-trips lit and unlit pixels back out of the IDAT", () => {
    const width = 37; // deliberately not a multiple of 8
    const height = 5;
    const lit = (x: number, y: number) => (x + y * 3) % 7 < 2;
    const bytes = encodeBilevelPng(image(width, height, lit));
    const idat = chunks(bytes).find((c) => c.type === "IDAT")!;

    // Undo the zlib wrapper: 2-byte header, then stored blocks, then adler32.
    const deflate = idat.data.subarray(2, idat.data.length - 4);
    const rowBytes = Math.ceil(width / 8);
    const expectedRawLength = (rowBytes + 1) * height;
    const raw = new Uint8Array(expectedRawLength);
    let read = 0;
    let written = 0;
    while (written < expectedRawLength) {
      // A stored deflate block is 1 header byte, then LEN (2, little-endian),
      // then NLEN (2, the complement), then LEN bytes of data.
      const header = deflate[read]!;
      expect(header & 0b110, "BTYPE must be 00 (stored)").toBe(0);
      const len = deflate[read + 1]! | (deflate[read + 2]! << 8);
      expect(deflate[read + 3]! | (deflate[read + 4]! << 8)).toBe(~len & 0xffff);
      const data = deflate.subarray(read + 5, read + 5 + len);
      expect(data.length).toBe(len);
      raw.set(data, written);
      read += 5 + len;
      written += len;
    }
    // The whole stream was consumed, so there is no truncated tail.
    expect(read).toBe(deflate.length);

    // Every scanline starts with filter byte 0, then bit-packed pixels.
    for (let y = 0; y < height; y += 1) {
      const rowStart = y * (rowBytes + 1);
      expect(raw[rowStart]).toBe(0);
      for (let x = 0; x < width; x += 1) {
        const bit = (raw[rowStart + 1 + (x >>> 3)]! & (0x80 >>> (x & 7))) !== 0;
        expect(bit).toBe(lit(x, y));
      }
    }
  });

  it("refuses a buffer whose pixels do not match its declared size", () => {
    expect(() =>
      encodeBilevelPng({ width: 8, height: 8, palette: PALETTE, pixels: new Uint8Array(3) }),
    ).toThrow(/expected 64/);
  });

  it("refuses a zero-size image", () => {
    expect(() =>
      encodeBilevelPng({ width: 0, height: 4, palette: PALETTE, pixels: new Uint8Array(0) }),
    ).toThrow(/positive size/);
  });
});

describe("toBase64", () => {
  it("matches the standard encoding, including padding", () => {
    expect(toBase64(new Uint8Array([0x66, 0x6f, 0x6f]))).toBe("Zm9v");
    expect(toBase64(new Uint8Array([0x66, 0x6f]))).toBe("Zm8=");
    expect(toBase64(new Uint8Array([0x66]))).toBe("Zg==");
  });

  it("round-trips a real PNG at a size a signature actually produces", () => {
    const bytes = encodeBilevelPng(image(64, 16, (x, y) => x === y));
    const encoded = toBase64(bytes);
    expect(encoded.length % 4).toBe(0);
    const decoded = atob(encoded);
    expect(decoded.length).toBe(bytes.length);
    for (let i = 0; i < bytes.length; i += 1) {
      expect(decoded.charCodeAt(i)).toBe(bytes[i]);
    }
  });
});
