import type { Summary, Tree } from "../src/api";

/** A record id whose digest is `name` repeated; readable in failure output. */
export function id(name: string): string {
  return `ppr1:${name.repeat(64).slice(0, 64)}`;
}

export function summary(name: string, position: number, parents: readonly string[] = []): Summary {
  return {
    id: id(name),
    position,
    kind: "example.note/v1",
    created: "2026-01-15T12:00:00Z",
    signer: `ed25519:${"a".repeat(64)}`,
    parents: parents.map(id),
    artifacts: 0,
    label: name,
  };
}

export function tree(
  record: Summary,
  ancestors: readonly Summary[] = [],
  descendants: readonly Summary[] = [],
  missing: readonly string[] = [],
): Tree {
  return { record, ancestors, descendants, missing: missing.map(id) };
}
