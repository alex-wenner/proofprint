import { describe, expect, it } from "vitest";
import { Router } from "../src/router";

describe("Router.parse", () => {
  it("reads the history filter from the query", () => {
    const route = Router.parse("/", "?kind=ml.dataset%2Fv1&offset=50");
    expect(route.name).toBe("history");
    if (route.name !== "history") return;
    expect(route.query.get("kind")).toBe("ml.dataset/v1");
    expect(route.query.get("offset")).toBe("50");
  });

  it("decodes record ids", () => {
    expect(Router.parse("/records/ppr1:abc", "")).toEqual({ name: "record", id: "ppr1:abc" });
    expect(Router.parse("/records/ppr1%3Aabc/", "")).toEqual({ name: "record", id: "ppr1:abc" });
  });

  it("does not throw on malformed escapes or unknown paths", () => {
    expect(Router.parse("/records/%E0%A4%A", "")).toEqual({ name: "unknown", path: "/records/%E0%A4%A" });
    expect(Router.parse("/records/a/b", "")).toEqual({ name: "unknown", path: "/records/a/b" });
    expect(Router.parse("/elsewhere", "")).toEqual({ name: "unknown", path: "/elsewhere" });
  });
});
