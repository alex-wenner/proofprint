#!/usr/bin/env python3
"""Train a small nanoGPT and record the run in a ProofPrint ledger as it goes.

This is a condensed, single-process copy of nanoGPT's `train.py` with the
training calls left as they are and a handful of publishing calls added: the
dataset, the run, each logged step, the checkpoint, the resulting model, and a
final evaluation are sent to a running `proofprint-node` while training runs.
The model is deliberately tiny so the whole example finishes on a CPU in under
a minute; the point is the record chain, not the text it produces.

    cargo run -p proofprint-node          # in another terminal
    python examples/nanogpt/train_recorded.py --nanogpt /path/to/nanoGPT

The script uses the `proofprint` package from `../../python` when it is not
installed, and reads the character data in `data/shakespeare_char` when that has
been prepared. Otherwise it trains on a short built-in corpus.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import sys
import time
from contextlib import nullcontext
from dataclasses import dataclass
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
SERVER = "http://127.0.0.1:4780"

BUILT_IN_CORPUS = (
    "The ledger keeps a record of each claim. A record names its ancestors, so a "
    "reader can walk back from a result to the data and the code that produced it. "
    "A signature shows who made a claim, not that the claim is true. The reader "
    "decides what to trust.\n"
) * 400


def find_nanogpt(explicit: str | None) -> Path:
    """Locate a nanoGPT checkout, from the argument, the environment, or nearby."""
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit))
    if os.environ.get("NANOGPT_DIR"):
        candidates.append(Path(os.environ["NANOGPT_DIR"]))
    candidates += [
        HERE / "nanoGPT",
        HERE.parent.parent / "nanoGPT",
        Path.cwd() / "nanoGPT",
        Path.cwd(),
    ]
    for candidate in candidates:
        if (candidate / "model.py").is_file() and (candidate / "train.py").is_file():
            return candidate.resolve()
    raise SystemExit(
        "nanoGPT not found. Clone it and pass --nanogpt /path/to/nanoGPT, "
        "or set NANOGPT_DIR."
    )


def import_proofprint() -> None:
    """Use the repository's Python package when it is not installed."""
    try:
        import proofprint  # noqa: F401
    except ImportError:
        sys.path.insert(0, str(HERE.parent.parent / "python"))


@dataclass
class CharData:
    train: np.ndarray
    val: np.ndarray
    vocab_size: int
    source: str
    files: list[Path]


def load_data(nanogpt_dir: Path, dataset: str, block_size: int) -> CharData:
    """Prepared nanoGPT character data when present, else a built-in corpus."""
    data_dir = nanogpt_dir / "data" / dataset
    meta, train_bin, val_bin = (
        data_dir / "meta.pkl",
        data_dir / "train.bin",
        data_dir / "val.bin",
    )
    if meta.is_file() and train_bin.is_file() and val_bin.is_file():
        import pickle

        info = pickle.loads(meta.read_bytes())
        vocab_size = int(info["vocab_size"])
        train = np.memmap(train_bin, dtype=np.uint16, mode="r")
        val = np.memmap(val_bin, dtype=np.uint16, mode="r")
        print(f"using {data_dir} ({len(train):,} train tokens, vocab {vocab_size})")
        return CharData(train, val, vocab_size, str(data_dir), [meta, train_bin, val_bin])

    text = BUILT_IN_CORPUS
    chars = sorted(set(text))
    stoi = {char: index for index, char in enumerate(chars)}
    ids = np.array([stoi[char] for char in text], dtype=np.uint16)
    split = int(len(ids) * 0.9)
    print(f"using the built-in corpus ({len(text):,} characters, vocab {len(chars)})")
    return CharData(ids[:split], ids[split:], len(chars), "built-in corpus", [])


def git_commit(nanogpt_dir: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", "-C", str(nanogpt_dir), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def write_dataset_manifest(out_dir: Path, data: CharData) -> Path:
    """A small JSON file listing the SHA-256 of every file the data came from."""
    files = [
        {"path": path.name, "sha256": sha256(path), "bytes": path.stat().st_size}
        for path in data.files
    ]
    manifest = {
        "source": data.source,
        "vocab_size": data.vocab_size,
        "train_tokens": int(len(data.train)),
        "val_tokens": int(len(data.val)),
        "files": files,
    }
    path = out_dir / "dataset.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def get_batch(data: np.ndarray, block_size: int, batch_size: int, device: str):
    import torch

    ix = np.random.randint(len(data) - block_size, size=(batch_size,))
    x = torch.stack([torch.from_numpy(data[i : i + block_size].astype(np.int64)) for i in ix])
    y = torch.stack([torch.from_numpy(data[i + 1 : i + 1 + block_size].astype(np.int64)) for i in ix])
    if device.startswith("cuda"):
        x, y = x.pin_memory().to(device, non_blocking=True), y.pin_memory().to(device, non_blocking=True)
    else:
        x, y = x.to(device), y.to(device)
    return x, y


def get_lr(step: int, learning_rate: float, warmup: int, decay: int, min_lr: float) -> float:
    if step < warmup:
        return learning_rate * (step + 1) / (warmup + 1)
    if step > decay:
        return min_lr
    ratio = (step - warmup) / (decay - warmup)
    coefficient = 0.5 * (1.0 + math.cos(math.pi * ratio))
    return min_lr + coefficient * (learning_rate - min_lr)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nanogpt", help="path to a nanoGPT checkout")
    parser.add_argument("--node", default=SERVER, help="proofprint-node base URL")
    parser.add_argument("--out", default=str(HERE / "out"), help="where checkpoints and reports are written")
    parser.add_argument("--dataset", default="shakespeare_char", help="name under nanoGPT's data/")
    parser.add_argument("--run-id", default=None, help="record id for the run; default includes the time")
    parser.add_argument("--max-iters", type=int, default=60)
    parser.add_argument("--eval-interval", type=int, default=20)
    parser.add_argument("--eval-iters", type=int, default=20)
    parser.add_argument("--log-interval", type=int, default=5)
    parser.add_argument("--batch-size", type=int, default=12)
    parser.add_argument("--block-size", type=int, default=64)
    parser.add_argument("--n-layer", type=int, default=4)
    parser.add_argument("--n-head", type=int, default=4)
    parser.add_argument("--n-embd", type=int, default=128)
    parser.add_argument("--learning-rate", type=float, default=1e-3)
    parser.add_argument("--min-lr", type=float, default=1e-4)
    parser.add_argument("--warmup-iters", type=int, default=10)
    parser.add_argument("--weight-decay", type=float, default=0.1)
    parser.add_argument("--device", default="cuda" if _cuda() else "cpu")
    parser.add_argument("--seed", type=int, default=1337)
    return parser.parse_args()


def _cuda() -> bool:
    try:
        import torch

        return torch.cuda.is_available()
    except ImportError:
        return False


def main() -> int:
    args = parse_args()
    import_proofprint()
    nanogpt_dir = find_nanogpt(args.nanogpt)
    sys.path.insert(0, str(nanogpt_dir))

    import torch
    from model import GPT, GPTConfig

    from proofprint import NodeClient, NodeError, Publisher, TrainingRecorder

    torch.manual_seed(args.seed)
    np.random.seed(args.seed)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    client = NodeClient(args.node)
    try:
        status = client.status()
    except NodeError as failure:
        print(f"No node at {args.node}: {failure}\nStart one with: cargo run -p proofprint-node")
        return 1
    print(f"node {args.node} holds {status.records} records, signer {status.signer}")

    data = load_data(nanogpt_dir, args.dataset, args.block_size)
    manifest_path = write_dataset_manifest(out_dir, data)

    device_type = "cuda" if "cuda" in args.device else "cpu"
    dtype = "bfloat16" if device_type == "cuda" and torch.cuda.is_bf16_supported() else "float32"
    ptdtype = {"float32": torch.float32, "bfloat16": torch.bfloat16, "float16": torch.float16}[dtype]
    ctx = nullcontext() if device_type == "cpu" else torch.amp.autocast(device_type=device_type, dtype=ptdtype)

    model_args = dict(
        n_layer=args.n_layer,
        n_head=args.n_head,
        n_embd=args.n_embd,
        block_size=args.block_size,
        bias=True,
        vocab_size=data.vocab_size,
        dropout=0.0,
    )
    model = GPT(GPTConfig(**model_args)).to(args.device)
    parameters = model.get_num_params()
    optimizer = model.configure_optimizers(
        args.weight_decay, args.learning_rate, (0.9, 0.95), device_type
    )

    @torch.no_grad()
    def estimate_loss() -> dict[str, float]:
        model.eval()
        out: dict[str, float] = {}
        for split, source in (("train", data.train), ("val", data.val)):
            losses = torch.zeros(args.eval_iters)
            for index in range(args.eval_iters):
                x, y = get_batch(source, args.block_size, args.batch_size, args.device)
                with ctx:
                    _, loss = model(x, y)
                losses[index] = loss.item()
            out[split] = losses.mean().item()
        model.train()
        return out

    commit = git_commit(nanogpt_dir)
    config = {
        "script": "examples/nanogpt/train_recorded.py",
        "dataset": args.dataset,
        "data_source": data.source,
        **model_args,
        "batch_size": args.batch_size,
        "max_iters": args.max_iters,
        "learning_rate": args.learning_rate,
        "min_lr": args.min_lr,
        "warmup_iters": args.warmup_iters,
        "weight_decay": args.weight_decay,
        "device": args.device,
        "dtype": dtype,
        "seed": args.seed,
    }
    run_id = args.run_id or f"nanogpt-{time.strftime('%Y%m%d-%H%M%S')}"

    publisher = Publisher(client)
    dataset = publisher.dataset(
        name=f"nanogpt-{args.dataset}",
        version="1.0.0",
        origin=data.source,
        license="built-in" if not data.files else None,
        examples=int(len(data.train)),
        manifest=manifest_path,
        notes={
            "vocab_size": data.vocab_size,
            "train_tokens": int(len(data.train)),
            "val_tokens": int(len(data.val)),
            "files_hashed": len(data.files),
        },
    )
    run = TrainingRecorder(
        client,
        run_id=run_id,
        framework=f"nanoGPT@{commit[:7] if commit else 'unknown'}/pytorch {torch.__version__}",
        config=config,
        code_commit=commit,
        environment={"python": sys.version.split()[0], "torch": torch.__version__, "device": args.device},
        total_steps=args.max_iters,
        parents=[dataset.id],
    )
    run.start()
    print(f"dataset {dataset.id}\nrun     {run.run}")

    recorded: set[int] = set()

    def record_step(step: int, loss: float, metrics: dict, checkpoint: Path | None = None) -> None:
        if step in recorded:
            return
        recorded.add(step)
        record = run.step(step, loss=loss, samples_seen=(step + 1) * args.batch_size * args.block_size,
                          metrics=metrics, checkpoint=checkpoint)
        print(f"  recorded step {step} as {record}")

    best_val = float("inf")
    checkpoint_path = out_dir / "ckpt.pt"
    tokens_per_iter = args.batch_size * args.block_size

    def save_checkpoint(step: int, val_loss: float) -> None:
        torch.save(
            {
                "model": model.state_dict(),
                "optimizer": optimizer.state_dict(),
                "model_args": model_args,
                "iter_num": step,
                "best_val_loss": val_loss,
                "config": config,
            },
            checkpoint_path,
        )

    print(f"training {args.max_iters} steps on {args.device}")
    start = time.time()
    for step in range(args.max_iters + 1):
        learning_rate = get_lr(step, args.learning_rate, args.warmup_iters, args.max_iters, args.min_lr)
        for group in optimizer.param_groups:
            group["lr"] = learning_rate

        if step % args.eval_interval == 0:
            losses = estimate_loss()
            best_val = min(best_val, losses["val"])
            save_checkpoint(step, best_val)
            print(f"step {step}: train {losses['train']:.4f}, val {losses['val']:.4f} (checkpoint saved)")
            record_step(
                step,
                losses["val"],
                {"train_loss": round(losses["train"], 4), "val_loss": round(losses["val"], 4),
                 "lr": learning_rate, "eval": True},
                checkpoint=checkpoint_path,
            )

        if step == args.max_iters:
            break

        x, y = get_batch(data.train, args.block_size, args.batch_size, args.device)
        with ctx:
            _, loss = model(x, y)
        optimizer.zero_grad(set_to_none=True)
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        optimizer.step()

        elapsed = time.time() - start
        if step % args.log_interval == 0:
            metrics = {
                "lr": learning_rate,
                "tokens_per_sec": round(((step + 1) * tokens_per_iter) / max(elapsed, 1e-6), 1),
                "elapsed_s": round(elapsed, 2),
            }
            print(f"iter {step}: loss {loss.item():.4f}")
            record_step(step, loss.item(), metrics)

    final = estimate_loss()
    save_checkpoint(args.max_iters, final["val"])
    record_step(
        args.max_iters,
        final["val"],
        {"train_loss": round(final["train"], 4), "val_loss": round(final["val"], 4), "final": True},
        checkpoint=checkpoint_path,
    )

    model_id = run.model(
        name=f"nanogpt-{args.dataset}",
        version="1.0.0",
        architecture="nanoGPT/GPT",
        parameters=parameters,
        weights=checkpoint_path,
        definition=nanogpt_dir / "model.py",
        notes={
            "model_args": model_args,
            "final_val_loss": round(final["val"], 4),
            "trained_seconds": round(time.time() - start, 2),
            "dataset_record": dataset.id,
        },
    )
    print(f"model   {model_id}")

    report_path = out_dir / "eval.json"
    report_path.write_text(
        json.dumps({"benchmark": args.dataset, "train_loss": final["train"], "val_loss": final["val"],
                    "steps": args.max_iters, "eval_iters": args.eval_iters}, indent=2) + "\n",
        encoding="utf-8",
    )
    evaluation = publisher.evaluation(
        f"nanoGPT-eval/1.0",
        args.dataset,
        {"train_loss": round(final["train"], 4), "val_loss": round(final["val"], 4)},
        report=report_path,
        parents=[model_id],
    )
    print(f"eval    {evaluation.id}")

    checked = client.verify(model_id)
    print(f"verify  signature={checked.signature} inclusion={checked.inclusion} files={len(checked.verified)}")

    print("\nlineage:")
    for record in reversed(list(client.walk_parents(model_id))):
        label = record.payload.get("title") or record.payload.get("name") or record.payload.get("run_id") or ""
        print(f"  {record.kind:32} {record.id} {label}")
    print(f"\nopen {args.node}/records/{model_id} in the explorer")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
