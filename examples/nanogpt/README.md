# Recording a nanoGPT run

[`train_recorded.py`](train_recorded.py) trains a small
[nanoGPT](https://github.com/karpathy/nanoGPT) and records the whole run in a
ProofPrint ledger while it trains. It is a condensed, single-process copy of
nanoGPT's `train.py`; the training calls are unchanged and a handful of
publishing calls have been added around them.

The model is deliberately tiny, so the example finishes on a CPU in under a
minute. The point is the chain of records, not the text the model produces.

## Run it

Start a node, which creates its signing key on first run:

```bash
cargo run -p proofprint-node
```

Then, in another terminal:

```bash
python examples/nanogpt/train_recorded.py --nanogpt /path/to/nanoGPT
```

The script finds nanoGPT through `--nanogpt`, the `NANOGPT_DIR` environment
variable, or a `nanoGPT` directory beside the repository. It uses the
`proofprint` package from `python/` when that is not installed, and it reads
`data/shakespeare_char` when you have prepared it with nanoGPT's own
`prepare.py`. Otherwise it trains on a short built-in corpus.

Useful flags: `--max-iters`, `--eval-interval`, `--log-interval`, `--device`,
and `--node` (default `http://127.0.0.1:4780`).

## What it publishes

| Order | Record | Holds |
| --- | --- | --- |
| 1 | `ml.dataset/v1` | The corpus, a vocab size, and a manifest of SHA-256 hashes for each source file |
| 2 | `ml.training-run/v1` | The model and optimizer config, the nanoGPT commit, the Python and torch versions |
| 3+ | `ml.training-step/v1` | One per logged step: loss, learning rate, tokens per second; the checkpoint is attached at each eval |
| last | `ml.model/v1` | The architecture, the parameter count, and the checkpoint and `model.py` as attachments |
| last | `ml.evaluation/v1` | The final train and val loss, with a JSON report attached |

Every step names the run and the previous step as parents, and the model names
the last step, so the whole run is one chain from the evaluation back to the
dataset. Open the printed URL, or the model's id under
<http://127.0.0.1:4780/records/>, to see the lineage, the payloads, the
attachments, and a re-check of the signature and file hashes.

```
$ python examples/nanogpt/train_recorded.py --nanogpt ~/nanoGPT
node http://127.0.0.1:4780 holds 0 records, signer ed25519:09dcbf48…a897c
using the built-in corpus (100,800 characters, vocab 26)
number of parameters: 0.80M
dataset ppr1:83b789a7…c69314
run     ppr1:5c4daa5b…95d8f8
training 40 steps on cuda
  recorded step 0 as ppr1:dc2b8e78…6a1b13e
  …
model   ppr1:28a25d15…f7f6016
eval    ppr1:9a6781b6…96f867
verify  signature=True inclusion=True files=2

open http://127.0.0.1:4780/records/ppr1:28a25d15… in the explorer
```

## Put it in nanoGPT's own `train.py`

The example copy exists so the wiring is visible in one file. In your own
`train.py`, the additions are:

1. After the model and optimizer exist, publish the dataset and open a run:

   ```python
   from proofprint import NodeClient, Publisher, TrainingRecorder

   client = NodeClient()
   dataset = Publisher(client).dataset("openwebtext", "1.0", origin="openwebtext",
                                       manifest="data/manifest.json")
   run = TrainingRecorder(client, run_id="nanogpt-1", framework="nanoGPT/pytorch",
                          config=config, total_steps=max_iters, parents=[dataset.id])
   run.start()
   ```

2. Inside the loop, at each log line:

   ```python
   run.step(iter_num, loss=lossf, metrics={"lr": lr, "mfu": running_mfu})
   ```

3. At each checkpoint, attach it to that step:

   ```python
   run.step(iter_num, loss=losses["val"], checkpoint=os.path.join(out_dir, "ckpt.pt"))
   ```

4. After the loop, publish the model and an evaluation:

   ```python
   model = run.model("nanogpt-gpt2", "1.0.0", "nanoGPT/GPT",
                     parameters=raw_model.get_num_params(), weights=os.path.join(out_dir, "ckpt.pt"),
                     definition=os.path.join("model.py"))
   Publisher(client).evaluation("nanoGPT-eval/1.0", "openwebtext",
                                {"val_loss": best_val_loss}, parents=[model])
   ```

## What the records do and do not show

These records show that the node's key signed these bytes, that the files match
their hashes, and what each record was built from. They do not show that the
training happened as stated. A signature is a claim about authorship, not
truth; checking that needs another party to re-run the work and publish what
they found. See [python/README.md](../../python/README.md) for that flow.
