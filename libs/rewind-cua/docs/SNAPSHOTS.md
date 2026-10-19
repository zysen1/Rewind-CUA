# Snapshot backends

Rewind-CUA checkpoints the workspace before a risky action and copies it
back when the verifier fails. Only the directory backend runs by default.
The others are the mechanisms upstream trycua/cua actually ships, so a
caller can point the command backend at them. This crate does not start a
VM, and it does not pretend that `Sandbox.suspend` is a stack of named
rewind points.

## directory

`DirWorld` copies `workspace/root` into `workspace/snaps/<id>` and, on
rewind, replaces `root` with that copy. The demo and the scripted eval use
this. Mail and payments are stored in an outbox outside the tree, so a
rewind does not unsend them. Symlinks are refused.

A full copy is portable. It is not an APFS clonefile or a btrfs snapshot.
Lume's own Linux-disk clone tries `cp -c` and falls back to a copy; that
logic stays in Lume.

## command

`CommandSnapshot` runs a program you supply. Capture and restore arguments
may contain `{label}`, `{root}`, and `{store}`. The unit test uses a shell
script that copies the directory. A wrapper around the QEMU or Lume
operations below would use the same seam. Nothing in this crate invokes
QEMU or Lume itself.

## qemu-savevm

In `libs/cua/crates/cua-vmm/src/qemu/mod.rs`:

- While the VM is running, `Runtime::checkpoint` opens QMP and calls
  `savevm`. The snapshot is inside the instance qcow2 and includes RAM,
  devices, and disks. `Runtime::restore` calls `loadvm`.
- While the VM is stopped, checkpoint freezes the disk into an immutable
  layer instead of taking a live snapshot.
- `suspend` is QMP `stop`. That pauses the VM. It is not a named checkpoint.

The Python bare-metal runtime
(`libs/python/cua-sandbox/cua_sandbox/runtime/qemu.py`) saves one slot
named `cua-snapshot` and resumes with `-loadvm cua-snapshot`. One slot is
not a stack of rewind points.

## lume-clone

In `libs/cua/crates/cua-vmm/src/lume/mod.rs`:

- Lume has no in-memory pause. `suspend` stops the VM.
- `Runtime::checkpoint` stops a running VM, clones it (`fork` /
  `clone_vm`), and starts it again if it was running.
- Linux disk VMs keep the disk outside Lume. The clone tries `cp -c`
  (APFS clonefile) and falls back to a full copy.

Restoring that checkpoint means booting the clone. It is not `loadvm`.

## sandbox-suspend

`Sandbox.snapshot()` in `libs/python/cua-sandbox/cua_sandbox/sandbox.py`
raises `CloudEnded`. Named cloud snapshots are gone.

`Sandbox.suspend` / `resume` is one power transition: a container pause, a
QEMU pause, or a Lume stop. Rewind-CUA does not use it as a checkpoint
stack.
