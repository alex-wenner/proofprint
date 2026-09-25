import { Component, type ReactNode } from "react";
import type { Api, Status } from "./api";
import { StatusBar } from "./components/status-bar";
import { Parties } from "./parties";
import { Router, type Route } from "./router";
import { HistoryView } from "./views/history";
import { MissingView } from "./views/missing";
import { RecordView } from "./views/record";

interface AppProps {
  readonly api: Api;
  readonly router: Router;
}

interface AppState {
  readonly route: Route;
  /** Increases on every navigation. Views are keyed by it, so revisiting a page reloads it. */
  readonly visit: number;
  readonly status: Status | null;
  readonly unreachable: boolean;
  /** The first status request has finished, so the node's own key is known if it can be. */
  readonly ready: boolean;
}

/** The page shell. Owns the route and the node status; the views own their data. */
export class App extends Component<AppProps, AppState> {
  override state: AppState = {
    route: Router.current(),
    visit: 0,
    status: null,
    unreachable: false,
    ready: false,
  };
  private readonly parties = new Parties();

  override componentDidMount(): void {
    this.props.router.start((route) => this.setState(({ visit }) => ({ route, visit: visit + 1 })));
    void this.refreshStatus();
  }

  override componentDidUpdate(_: AppProps, previous: AppState): void {
    if (previous.visit !== this.state.visit) {
      window.scrollTo(0, 0);
      void this.refreshStatus();
    }
  }

  override componentWillUnmount(): void {
    this.props.router.stop();
  }

  override render(): ReactNode {
    const { status, unreachable, ready } = this.state;
    return (
      <>
        <header className="top">
          <div className="top-inner">
            <a href="/" className="brand">
              ProofPrint
            </a>
            <StatusBar status={status} unreachable={unreachable} />
          </div>
        </header>
        <main className="page" id="main">
          {ready ? this.view() : null}
        </main>
      </>
    );
  }

  private view(): ReactNode {
    const { api } = this.props;
    const { route, visit } = this.state;
    switch (route.name) {
      case "history":
        return <HistoryView key={visit} api={api} parties={this.parties} query={route.query} navigate={this.navigate} />;
      case "record":
        return <RecordView key={visit} api={api} parties={this.parties} id={route.id} />;
      case "unknown":
        return <MissingView key={visit} path={route.path} />;
    }
  }

  private readonly navigate = (path: string): void => {
    this.props.router.navigate(path);
  };

  private async refreshStatus(): Promise<void> {
    try {
      const status = await this.props.api.status();
      this.parties.claim(status.signer);
      this.setState({ status, unreachable: false, ready: true });
    } catch {
      this.setState({ unreachable: true, ready: true });
    }
  }
}
