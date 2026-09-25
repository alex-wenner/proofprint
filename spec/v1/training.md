# Training records — experimental v1

How a training run is recorded so that a gap in the record can be found by
someone who only has the records. Record encoding, signing, and ids follow
[record.md](record.md).

## Weight manifests

A weight manifest describes every tensor in a model state:

```json
{"tensors":[{"dtype":"F32","name":"layer.weight","sha256":"<64 hex>","shape":[4,3]}]}
```

- `tensors` is sorted by `name` in UTF-8 byte order. Names are unique.
- `dtype` uses safetensors names: `BOOL`, `U8`, `I8`, `U16`, `I16`, `U32`,
  `I32`, `U64`, `I64`, `F16`, `BF16`, `F32`, `F64`, `F8_E4M3`, `F8_E5M2`.
- `shape` lists the dimensions; a scalar has `[]`.
- `sha256` is the lowercase hex SHA-256 of the tensor's raw little-endian
  bytes in row-major order.

The manifest's bytes are its canonical JSON under the rules in record.md. Its
digest is `sha256:` followed by the lowercase hex SHA-256 of those bytes. Two
states with the same digest hold bit-identical tensors under the same names.

`vectors.json` includes one manifest and its digest.

A state held in memory is described by every entry of the framework's state
dictionary, including buffers. A state stored in `.safetensors` files is
described by the tensors in those files. The two can differ in names and in
duplicates of shared tensors, which is why releases are compared tensor by
tensor rather than by digest.

## Input digests

For each optimizer step, the recorder hashes the inputs of every forward call
made in training mode since the previous step. Each forward call contributes
the tensors among its positional arguments, then its keyword arguments sorted
by name, walking lists, tuples, and dictionaries in order. For each tensor:
its dtype name, its shape as decimal numbers joined by `x`, and its raw bytes.
The step's input digest is `sha256:` plus the hex SHA-256 over all of that.

A segment's `inputs` digest is `sha256:` plus the hex SHA-256 of the step input
digests joined by `\n`, with `-` for a step whose inputs were not observed.

Input digests commit to the data order without revealing the data. Anyone who
holds the data can rebuild each batch and compare.

## `ml.training-run/v1`

Fields in addition to those in the bundled schema:

| Field | Meaning |
| --- | --- |
| `initial_weights` | Manifest digest of the weights when recording started |
| `code` | `{"entry": path, "files": {path: "sha256:<hex>"}}` for the entry script and the project's own modules loaded at start |
| `recorder` | Name and version of the software that observed the run |

A run that continues training an existing model names that model's record as a
parent. The model's `weights_digest` should equal `initial_weights`.

## `ml.training-segment/v1`

One record per stretch of training. Parents: the run, and the previous segment
when there is one.

| Field | Meaning |
| --- | --- |
| `run_id` | The run's `run_id` |
| `steps_before`, `steps_after` | Optimizer steps on the model, counted from the start of the run, before and after this segment |
| `weights_before`, `weights_after` | Manifest digests at the start and end of the segment |
| `inputs` | Input digest over the segment's steps, as above |
| `steps_without_inputs` | Steps for which no training-mode forward call was observed |
| `other_optimizer_steps` | Optimizer steps in the same process that did not update this model |
| `weight_loads` | Times the model's state was loaded from elsewhere during the segment |
| `seconds` | Wall-clock time the segment covered |
| `device` | The hardware the model was on, e.g. `cuda: 8 x NVIDIA H100 80GB HBM3` |

Attachments:

| Name | Content |
| --- | --- |
| `steps` | `{"steps":[{"step":n,"inputs":"sha256:…"|null,"values":{...}}]}`, one entry per optimizer step |
| `weights-after` | The manifest whose digest is `weights_after` |
| `weights-before` | The manifest whose digest is `weights_before`, on the first segment a process records |
| `replay-state` | Optional. Model, optimizer, and random-number state at the end of the segment, enough to replay the next one |
| other names | Optional checkpoint files |

A segment is published only when it covers at least one step or the weights
changed.

## `ml.model/v1`

Additional field `weights_digest`: the manifest digest of the model's weights
in memory when the record was made. The `weights-manifest` attachment holds
that manifest. A model produced by a run names the segment it was taken after
as a parent.

## Continuity

Given a run, a segment, or a model, a checker follows the segment chain from
the start of the run to that point and reports:

| Finding | Severity | Condition |
| --- | --- | --- |
| Steps missing | gap | `steps_before` of a segment exceeds `steps_after` of the one before it, or the first segment does not start at 0 |
| Steps overlap | gap | `steps_before` of a segment is less than `steps_after` of the one before it |
| Weights changed | gap | `weights_before` of a segment differs from `weights_after` of the one before it, or the first segment's differs from the run's `initial_weights` |
| Base model differs | gap | A parent model's `weights_digest` differs from the run's `initial_weights` |
| Model differs | gap | A model's `weights_digest` differs from `weights_after` of the segment it names |
| Manifest does not match | gap | An attached manifest does not hash to the digest the record states |
| Two histories | gap | Two segments of the run both follow the same point |
| Replay failed | gap | A `segment-replay` verification of a segment has outcome `fail` |
| Weights loaded | warning | `weight_loads` > 0 and the result is not a state recorded earlier in the run |
| Inputs not observed | warning | `steps_without_inputs` > 0 |
| Cannot check | warning | Required fields are missing or malformed |
| Weights restored | note | `weight_loads` > 0 and the result equals a state recorded earlier in the run |
| Other training | note | `other_optimizer_steps` > 0 |
| Replays | note | How many segments have a `segment-replay` verification |

A chain with no gaps shows that every recorded change to the weights falls
inside a segment. It does not show that a segment's steps were computed as
recorded. A replay checks that.

## Replay

A replay starts from the `replay-state` of the previous segment (for the first
segment, the run's `replay-state`), confirms that its weights match
`weights_before`, feeds the recorded number of steps with inputs whose digests
match the recorded ones, and compares the result with `weights_after`.

The result is an `attestation.verification/v1` record with method
`segment-replay`, the segment as its parent, and the comparison attached as
`evidence`:

- `pass`: the replayed weights equal `weights_after`, or are within the stated
  tolerance of the segment's stored end state.
- `fail`: an input digest differs, or the weights end outside tolerance.
- `inconclusive`: the start state does not match `weights_before`, or there
  is nothing stored to measure a non-identical result against.

Exact replays need the same hardware, library versions, and deterministic
algorithms. Otherwise the result is measured as the distance from the recorded
end state, relative to the size of the segment's update.

## What a record cannot show

All of the above runs inside the process being recorded. Whoever controls that
process can remove the hooks or write weights directly, and a segment can hold
anything its signer chose to put in it. Replays catch invented segments.
Hardware attestation of the recording process is out of scope for this
version. Segments state their device and wall-clock time so they can be
compared with an operator's own compute records.
