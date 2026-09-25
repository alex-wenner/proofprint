import { Component, type FormEvent, type ReactNode } from "react";
import type { Api, RecordPage, Summary } from "../api";
import { Loader } from "../components/loader";
import { SignerChip, Title } from "../components/parts";
import { plural, recordPath, shortHash, shortId, timestamp } from "../format";
import type { Parties } from "../parties";

const PAGE_SIZE = 50;

export interface Filter {
  readonly kind: string;
  readonly signer: string;
  readonly offset: number;
}

export function readFilter(query: URLSearchParams): Filter {
  return {
    kind: query.get("kind")?.trim() ?? "",
    signer: query.get("signer")?.trim() ?? "",
    offset: Math.max(0, Math.floor(Number(query.get("offset")) || 0)),
  };
}

/** The history URL for a filter. Empty fields are left out. */
export function historyPath(filter: Filter): string {
  const params = new URLSearchParams();
  if (filter.kind) params.set("kind", filter.kind);
  if (filter.signer) params.set("signer", filter.signer);
  if (filter.offset > 0) params.set("offset", String(filter.offset));
  const search = params.toString();
  return search ? `/?${search}` : "/";
}

interface HistoryViewProps {
  readonly api: Api;
  readonly parties: Parties;
  readonly query: URLSearchParams;
  readonly navigate: (path: string) => void;
}

/** The log in append order, filterable by kind and signing key. */
export class HistoryView extends Component<HistoryViewProps> {
  override render(): ReactNode {
    const { api, parties, query, navigate } = this.props;
    const filter = readFilter(query);
    const load = () => api.records({ ...filter, limit: PAGE_SIZE });
    return (
      <div className="view">
        <Loader load={load}>
          {(page) => <HistoryPage page={page} filter={filter} parties={parties} navigate={navigate} />}
        </Loader>
      </div>
    );
  }
}

interface HistoryPageProps {
  readonly page: RecordPage;
  readonly filter: Filter;
  readonly parties: Parties;
  readonly navigate: (path: string) => void;
}

export class HistoryPage extends Component<HistoryPageProps> {
  override render(): ReactNode {
    const { page, filter, parties, navigate } = this.props;
    const filtered = filter.kind !== "" || filter.signer !== "";
    return (
      <>
        <Title text="History" />
        <h1>History</h1>
        <p className="muted">
          {filtered ? `${plural(page.total, "record")} match. ` : `${plural(page.total, "record")} in append order. `}
          {page.root ? (
            <span title={page.root}>
              Root <code>{shortHash(page.root)}</code>.
            </span>
          ) : null}
        </p>
        <FilterForm
          filter={filter}
          kinds={unique(page.records.map((record) => record.kind))}
          signers={unique(page.records.map((record) => record.signer))}
          navigate={navigate}
        />
        {page.records.length > 0 ? (
          <RecordTable records={page.records} filter={filter} parties={parties} />
        ) : (
          <EmptyHistory filtered={filtered || filter.offset > 0} />
        )}
        <Pager page={page} filter={filter} />
      </>
    );
  }
}

interface FilterFormProps {
  readonly filter: Filter;
  readonly kinds: readonly string[];
  readonly signers: readonly string[];
  readonly navigate: (path: string) => void;
}

interface FilterFormState {
  readonly kind: string;
  readonly signer: string;
}

export class FilterForm extends Component<FilterFormProps, FilterFormState> {
  override state: FilterFormState = { kind: this.props.filter.kind, signer: this.props.filter.signer };

  override render(): ReactNode {
    const { filter, kinds, signers } = this.props;
    return (
      <form className="filters" role="search" onSubmit={this.submit}>
        <label>
          <span>Kind</span>
          <input
            name="kind"
            list="kinds"
            value={this.state.kind}
            placeholder="ml.training-step/v1"
            autoComplete="off"
            onChange={(event) => this.setState({ kind: event.target.value })}
          />
        </label>
        <label>
          <span>Signer</span>
          <input
            name="signer"
            list="signers"
            value={this.state.signer}
            placeholder="ed25519:…"
            autoComplete="off"
            onChange={(event) => this.setState({ signer: event.target.value })}
          />
        </label>
        <datalist id="kinds">
          {kinds.map((value) => (
            <option key={value} value={value} />
          ))}
        </datalist>
        <datalist id="signers">
          {signers.map((value) => (
            <option key={value} value={value} />
          ))}
        </datalist>
        <div className="filter-actions">
          <button type="submit" className="button">
            Filter
          </button>
          {filter.kind || filter.signer ? (
            <a href="/" className="button quiet">
              Clear
            </a>
          ) : null}
        </div>
      </form>
    );
  }

  private readonly submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    this.props.navigate(historyPath({ kind: this.state.kind.trim(), signer: this.state.signer.trim(), offset: 0 }));
  };
}

interface RecordTableProps {
  readonly records: readonly Summary[];
  readonly filter: Filter;
  readonly parties: Parties;
}

class RecordTable extends Component<RecordTableProps> {
  override render(): ReactNode {
    const { records, filter, parties } = this.props;
    return (
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              {["#", "Record", "Kind", "Signer", "Created", "Parents", "Files"].map((heading) => (
                <th key={heading} scope="col">
                  {heading}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <tr key={record.id}>
                <td className="num muted">{record.position}</td>
                <td>
                  <a href={recordPath(record.id)} className="record-link">
                    {record.label}
                  </a>
                  <div className="id muted">{shortId(record.id)}</div>
                </td>
                <td>
                  <a href={historyPath({ ...filter, kind: record.kind, offset: 0 })} className="kind" title="Show only this kind">
                    {record.kind}
                  </a>
                </td>
                <td>
                  <SignerChip parties={parties} signer={record.signer} href={historyPath({ ...filter, signer: record.signer, offset: 0 })} />
                </td>
                <td className="nowrap">{timestamp(record.created)}</td>
                <td className="num">{record.parents.length}</td>
                <td className="num">{record.artifacts}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
}

class EmptyHistory extends Component<{ readonly filtered: boolean }> {
  override render(): ReactNode {
    if (this.props.filtered) return <p className="empty">No records match this filter.</p>;
    return (
      <div className="empty">
        <p>This log is empty.</p>
        <p className="muted">
          Publish with the CLI or the Python client, or restart the node with --demo to load example records.
        </p>
      </div>
    );
  }
}

interface PagerProps {
  readonly page: RecordPage;
  readonly filter: Filter;
}

class Pager extends Component<PagerProps> {
  override render(): ReactNode {
    const { page, filter } = this.props;
    if (page.total <= PAGE_SIZE && page.offset === 0) return null;
    const first = page.records.length === 0 ? 0 : page.offset + 1;
    const last = page.offset + page.records.length;
    const next = page.offset + PAGE_SIZE;
    return (
      <nav className="pager" aria-label="Pages">
        {page.offset > 0 ? (
          <a href={historyPath({ ...filter, offset: Math.max(0, page.offset - PAGE_SIZE) })} className="button quiet">
            Previous
          </a>
        ) : null}
        <span className="muted">{`${first}–${last} of ${page.total}`}</span>
        {next < page.total ? (
          <a href={historyPath({ ...filter, offset: next })} className="button quiet">
            Next
          </a>
        ) : null}
      </nav>
    );
  }
}

function unique(values: readonly string[]): string[] {
  return [...new Set(values)].sort();
}
