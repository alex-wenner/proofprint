import { Component, type ReactNode } from "react";
import { ApiError, messageOf } from "../api";
import { Title } from "./parts";

type LoaderState<T> =
  | { readonly phase: "loading" }
  | { readonly phase: "ready"; readonly data: T }
  | { readonly phase: "failed"; readonly error: unknown };

interface LoaderProps<T> {
  readonly load: () => Promise<T>;
  readonly children: (data: T) => ReactNode;
}

/** Runs `load` once when mounted and renders the result. Give it a new `key` to load again. */
export class Loader<T> extends Component<LoaderProps<T>, LoaderState<T>> {
  override state: LoaderState<T> = { phase: "loading" };
  /** Bumped on unmount so a late answer from an earlier mount is dropped. */
  private request = 0;

  override componentDidMount(): void {
    const request = ++this.request;
    this.props.load().then(
      (data) => {
        if (request === this.request) this.setState({ phase: "ready", data });
      },
      (error: unknown) => {
        if (request === this.request) this.setState({ phase: "failed", error });
      },
    );
  }

  override componentWillUnmount(): void {
    this.request += 1;
  }

  override render(): ReactNode {
    switch (this.state.phase) {
      case "loading":
        return (
          <p className="muted" aria-busy="true">
            Loading…
          </p>
        );
      case "failed":
        return <Failure error={this.state.error} />;
      case "ready":
        return this.props.children(this.state.data);
    }
  }
}

/** Explains a failed load in terms of what to do next. */
export class Failure extends Component<{ readonly error: unknown }> {
  override render(): ReactNode {
    const { error } = this.props;
    if (error instanceof ApiError && error.status === 404) {
      return <Notice heading="Not in this log" text="This node does not store a record with that id." />;
    }
    if (error instanceof ApiError && error.status === 0) {
      return (
        <Notice heading="Node not reachable" text="Start it from the repository root, then reload:">
          <pre>cargo run -p proofprint-node -- --demo</pre>
        </Notice>
      );
    }
    return <Notice heading="Could not load this page" text={messageOf(error)} />;
  }
}

interface NoticeProps {
  readonly heading: string;
  readonly text: string;
  readonly children?: ReactNode;
}

export class Notice extends Component<NoticeProps> {
  override render(): ReactNode {
    const { heading, text, children } = this.props;
    return (
      <>
        <Title text={heading} />
        <h1>{heading}</h1>
        <p className="muted">{text}</p>
        {children}
        <p>
          <a href="/">Back to the history</a>
        </p>
      </>
    );
  }
}
