// Small pieces shared by several views.

import { Component, type ReactNode } from "react";
import { shortId } from "../format";
import type { Parties } from "../parties";

interface SectionProps {
  readonly heading: string;
  readonly children?: ReactNode;
}

export class Section extends Component<SectionProps> {
  override render(): ReactNode {
    return (
      <section className="panel">
        <h2>{this.props.heading}</h2>
        {this.props.children}
      </section>
    );
  }
}

interface FactProps {
  readonly term: string;
  readonly children?: ReactNode;
}

/** A term and its description, for use inside a <dl>. */
export class Fact extends Component<FactProps> {
  override render(): ReactNode {
    return (
      <>
        <dt>{this.props.term}</dt>
        <dd>{this.props.children}</dd>
      </>
    );
  }
}

interface SignerChipProps {
  readonly parties: Parties;
  readonly signer: string;
  /** Where the chip links to, usually the history filtered by this key. */
  readonly href?: string;
}

/** A signing key, coloured by party. */
export class SignerChip extends Component<SignerChipProps> {
  override render(): ReactNode {
    const { parties, signer, href } = this.props;
    const className = `chip ${parties.className(signer)}`;
    return href ? (
      <a className={className} title={signer} href={href}>
        {shortId(signer)}
      </a>
    ) : (
      <span className={className} title={signer}>
        {shortId(signer)}
      </span>
    );
  }
}

interface CopyButtonProps {
  readonly text: string;
  readonly label?: string;
}

interface CopyButtonState {
  /** Shown in place of the label for a moment after a copy. */
  readonly note: string | null;
}

export class CopyButton extends Component<CopyButtonProps, CopyButtonState> {
  override state: CopyButtonState = { note: null };
  private timer: number | undefined;

  override componentWillUnmount(): void {
    window.clearTimeout(this.timer);
  }

  override render(): ReactNode {
    return (
      <button type="button" className="button quiet" onClick={this.copy}>
        {this.state.note ?? this.props.label ?? "Copy"}
      </button>
    );
  }

  private readonly copy = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(this.props.text);
      this.flash("Copied");
    } catch {
      this.flash("Copy failed");
    }
  };

  private flash(note: string): void {
    this.setState({ note });
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.setState({ note: null }), 1500);
  }
}

/** Sets the document title while mounted. Renders nothing. */
export class Title extends Component<{ readonly text: string }> {
  override componentDidMount(): void {
    this.apply();
  }

  override componentDidUpdate(): void {
    this.apply();
  }

  override render(): ReactNode {
    return null;
  }

  private apply(): void {
    document.title = `${this.props.text} · ProofPrint`;
  }
}
