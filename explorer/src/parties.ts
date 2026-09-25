const PALETTE_SIZE = 6;

/**
 * Colour slots for signing keys, handed out in the order keys are first seen
 * and kept for the whole session. The node's own key, when known, is seen first.
 */
export class Parties {
  private readonly slots = new Map<string, number>();
  private own: string | null = null;

  /** Record which key belongs to this node. Only the first call has an effect. */
  claim(signer: string | null): void {
    if (signer === null || this.own !== null) return;
    this.own = signer;
    this.slot(signer);
  }

  isOwn(signer: string): boolean {
    return signer === this.own;
  }

  slot(signer: string): number {
    let slot = this.slots.get(signer);
    if (slot === undefined) {
      slot = this.slots.size % PALETTE_SIZE;
      this.slots.set(signer, slot);
    }
    return slot;
  }

  className(signer: string): string {
    return `party-${this.slot(signer)}`;
  }
}
