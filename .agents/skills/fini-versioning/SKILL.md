---
name: fini-versioning
description: "Process for changes to Fini app version metadata and version invariants. Use when changing package versions, About version display, CLI version output, Android version naming, Tauri/Cargo/npm metadata, or CI release version sync. For operational releases, release tags, and release workflow runbooks, use fini-release and load this skill only for metadata semantics. Depends on fini-scripting when implementation touches Makefile, npm scripts, cargo xtask, or CI command architecture."
---

# Fini versioning

Use this skill when work touches app version metadata, package manifests,
visible app version labels, CLI version output, Android versioning, or CI
release version sync. Use `fini-release` for operational releases. If the
implementation changes Makefile targets, npm scripts, `cargo xtask`, or CI
command architecture, also follow `fini-scripting`.

The invariants, the version surfaces, the release and automation boundaries,
and Android versioning are in `docs/release.md`, "Versioning".

1. Read `docs/release.md`, "Versioning", and find each version surface that
   the change touches.
2. Put version metadata mutation in `cargo xtask`.
3. Trace the write path, persisted metadata, and read path before claiming
   the version is fixed. Collect evidence for:
   - Write path: command or code that updates version metadata.
   - Persisted data: exact files and fields changed.
   - Read path: About UI, CLI version, Tauri metadata, or Android metadata as applicable.
   - Release safety: defer operational branch, tag, push, and CI workflow evidence to `fini-release`.
4. Use safe checks first. Do not create commits, push branches, or push tags
   unless the user explicitly requested a real release.
