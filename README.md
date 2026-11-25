# Blender Render Queue

[![Build](https://github.com/JckHoe/blender-tools/actions/workflows/build.yml/badge.svg)](https://github.com/JckHoe/blender-tools/actions/workflows/build.yml)
[![Latest release](https://img.shields.io/github/v/release/JckHoe/blender-tools)](https://github.com/JckHoe/blender-tools/releases/latest)

Small desktop app (Rust + egui) for queueing Blender command-line renders. Add several `.blend` files as jobs and render them in parallel, each in its own Blender process.

```
Blender exe [.................] Browse        ← shared by all jobs
┌──────────────┬───────────────────────────────┐
│ ● shot_A     │ .blend [...] Browse  Load     │
│ ○ shot_B     │ scenes table  ➕ Add scene    │
│ ✔ shot_C     │ ▶ Render ⏹ Stop   🗑 Remove   │
│ ➕ New job   │ log for this job              │
└──────────────┴───────────────────────────────┘
```

## Features

- **Multiple jobs**: one job per `.blend` file, each with its own scene list, log and Render/Stop.
- **Parallel renders**: every started job runs its own Blender process; starting one never waits on another.
- **Render all**: pick jobs in a popup (scenes and frame totals shown, finished jobs unchecked), then they render one after another, for unattended batches (e.g. over a weekend). Failed jobs don't stop the queue. **Stop all** cancels everything.
- **Scene loading**: picking a `.blend` reads its scenes and frame ranges automatically. "Load scenes" refreshes frame ranges and adds new scenes, keeping your per-scene settings.
- **Per-scene options**: enable/disable, frame range, optional output folder.
- **Status at a glance**: ○ idle, … reading scenes, ⏳ queued, ● rendering, ✔ done, ✖ failed, ⏹ cancelled.
- **Autosave**: settings are saved to `render-queue.json` next to the exe. Old single-file configs from v0.1.x are migrated to the first job.

Scenes within one job render one after another. Running many jobs at once shares CPU/GPU/RAM, so start only as many as your machine can handle.

## Usage

1. Set **Blender exe** (e.g. `C:\Program Files\Blender Foundation\Blender 4.x\blender.exe`).
2. Click **➕ New job** and pick a `.blend` file. Its scenes are loaded automatically.
3. Adjust scenes and output folders, then click **▶ Render**.
4. Add more jobs and render them while the first is still running, or click **▶ Render all** to render them back to back.

Each scene is rendered with:

```
blender -b <file.blend> -S <scene> -s <start> -e <end> [-o <output>] -a
```

## Download

Windows builds are attached to [GitHub releases](https://github.com/JckHoe/blender-tools/releases). Pushing a `v*` tag builds and publishes a release via CI.

## Build

```
cargo build --release
cargo test
```

Windows builds link the C runtime statically (`.cargo/config.toml`), so the exe runs without extra installs.

## Sample file

`examples/sample.blend` has two small scenes (Cycles and Eevee) for testing. Regenerate it with:

```
blender -b --python examples/make_sample.py
```
