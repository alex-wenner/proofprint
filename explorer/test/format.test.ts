import { describe, expect, it } from "vitest";
import { bytes, plural, recordPath, shortHash, shortId, timestamp, truncate } from "../src/format";
import { Parties } from "../src/parties";

describe("format", () => {
  it("shortens ids but keeps their prefix", () => {
    const record = `ppr1:${"0123456789abcdef".repeat(4)}`;
    expect(shortId(record)).toBe("ppr1:01234567…cdef");
    expect(shortId("ppr1:abc")).toBe("ppr1:abc");
    expect(shortHash("f".repeat(64))).toBe("ffffffff…ffff");
  });

  it("uses binary units", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1023)).toBe("1023 B");
    expect(bytes(1024)).toBe("1.0 KiB");
    expect(bytes(1536)).toBe("1.5 KiB");
    expect(bytes(20 * 1024 * 1024)).toBe("20 MiB");
    expect(bytes(5 * 1024 ** 5)).toBe("5120 TiB");
  });

  it("shows timestamps in UTC without converting them", () => {
    expect(timestamp("2026-01-15T12:00:00Z")).toBe("2026-01-15 12:00:00 UTC");
  });

  it("truncates by code point", () => {
    expect(truncate("short", 10)).toBe("short");
    expect(truncate("🙂🙂🙂🙂", 3)).toBe("🙂🙂…");
  });

  it("builds record paths and plurals", () => {
    expect(recordPath("ppr1:ab")).toBe("/records/ppr1:ab");
    expect(plural(1, "record")).toBe("1 record");
    expect(plural(2, "record")).toBe("2 records");
  });
});

describe("Parties", () => {
  it("gives the node's own key the first slot and others in order of first sight", () => {
    const parties = new Parties();
    parties.claim("own");
    parties.claim("someone-else");
    expect(parties.isOwn("own")).toBe(true);
    expect(parties.isOwn("someone-else")).toBe(false);
    expect(parties.slot("own")).toBe(0);
    expect(parties.slot("b")).toBe(1);
    expect(parties.slot("c")).toBe(2);
    expect(parties.slot("b")).toBe(1);
    expect(parties.className("c")).toBe("party-2");
  });

  it("reuses colours after six keys", () => {
    const parties = new Parties();
    const slots = ["a", "b", "c", "d", "e", "f", "g"].map((key) => parties.slot(key));
    expect(slots).toEqual([0, 1, 2, 3, 4, 5, 0]);
  });
});
