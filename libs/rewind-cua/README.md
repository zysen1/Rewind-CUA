# Rewind-CUA

**Built on [trycua/cua](https://github.com/trycua/cua) (MIT).**

An append-only journal, checkpoints, and a policy gate for computer-use agents. When the executor deletes the wrong file, Rewind-CUA rolls the workspace back and retries. When it is unsure, or the action is mail or a payment, it asks a stronger model or a person first.

可回滚、会举手的电脑操作 harness。执行模型删错文件时，Rewind-CUA 把工作区倒带回去再试。它没把握，或者动作是发信、付款时，先问更强的模型或人。

This directory is the extension. The rest of the repository is upstream Cua. See the root [NOTICE](../../NOTICE) and [REWIND.md](../../REWIND.md).

## Quickstart

Rust 1.83 or newer:

```bash
cd libs/rewind-cua
cargo test --locked --workspace
cargo run --locked --quiet -- replay demo/mistaken-delete.rewind.jsonl
make demo
```

`make demo` refreshes `demo/mistaken-delete.gif`.

The scripted scores are in [eval/results.json](eval/results.json). Cua-Bench and OSWorld are WIP: they were not run.

Snapshot behavior, including what upstream QEMU and Lume actually support, is in [docs/SNAPSHOTS.md](docs/SNAPSHOTS.md).
