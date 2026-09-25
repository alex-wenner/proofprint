# proofprint (Python)

Client for a running `proofprint-node`. Standard library only.

The node holds the signing key. This package builds records, uploads files,
and asks the node to sign and append. It never handles a key.

## Install

From the repository root:

```bash
pip install -e python
```

Add the `transformers` extra to use the Trainer callback:

```bash
pip install -e "python[transformers]"
```

## Start a node

```bash
cargo run -p proofprint-node
```

It listens on `127.0.0.1:4780` and creates `.proofprint/keys/default.key` on
first start. Everything below talks to that address unless you pass another
one to `NodeClient`.

## Record a training run

```python
from proofprint import NodeClient, Publisher, TrainingRecorder

client = NodeClient()
publisher = Publisher(client)

dataset = publisher.dataset(
    "support-tickets",
    "2026.09",
    origin="export",
    license="internal",
    examples=48_210,
    manifest="data/manifest.txt",
)

run = TrainingRecorder(
    client,
    run_id="ft-2026-09-19",
    framework="pytorch/2.8",
    config={"lr": 3e-4, "batch_size": 64},
    code_commit="4f2a9c1",
    parents=[dataset.id],
)
for step, loss in train():
    run.step(step, loss=loss)
run.step(final_step, loss=final_loss, checkpoint="out/model.safetensors")
model = run.model("ticket-router", "1.0.0", "transformer-encoder", weights="out/model.safetensors")
```

Every step names the run and the previous step as parents, so the chain can
be walked from the model back to the dataset:

```python
for record in client.walk_parents(model):
    print(record.kind, record.payload)
```

## Record an evaluation, and someone else's check of it

```python
evaluation = publisher.evaluation(
    "lm-eval-harness/0.4.8",
    "mmlu",
    {"accuracy": 0.612},
    report="results/mmlu.json",
    parents=[model],
)
```

A second party runs their own node and publishes a verification that names
your evaluation as its parent:

```python
Publisher(their_client).verification(
    "re-evaluation",
    "pass",
    detail="0.609 on our hardware, within the stated noise",
    evidence="their_results.json",
    parents=[evaluation.id],
)
```

`parents` must already be in the log the record is published to, so send your
evaluation and everything it descends from to their node first. The records
travel unchanged with your signatures, and their node checks them:

```python
client.copy_to(their_client, evaluation.id)
```

## Hugging Face Trainer

```python
from proofprint import NodeClient
from proofprint.trainer import RecordingCallback

trainer = Trainer(
    ...,
    callbacks=[RecordingCallback(NodeClient(), run_id="ft-2026-09-19", parents=[dataset.id])],
)
```

The callback publishes one run record at train start, with the training
arguments as its config. Each logged step becomes a step record. When a
checkpoint is saved at a logged step, its weights file is uploaded and
attached to that step's record.

## What a record shows

A published record shows that the node's key signed these bytes at this
position in this log. It does not show that the training happened, that the
dataset was used, or that the score is right. Those need a second party to
check and publish what they found.

## Tests

```bash
cd python
python -m unittest discover -s tests -t .
```

The end-to-end test runs only when `PROOFPRINT_NODE_BIN` points at a built
node binary, such as `../target/debug/proofprint-node`.
