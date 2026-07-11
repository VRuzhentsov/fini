# Cross-Distribution AppImage Compatibility

## Outcome

Fini distributes one AppImage per supported CPU architecture that launches with a rendered UI across the supported glibc Linux desktop matrix. The artifact must not couple bundled low-level desktop runtime ABI libraries to an incompatible host Mesa, EGL, Wayland, or GStreamer stack.

## Supported contract

- **Architectures:** x86_64 and aarch64.
- **Runtime family:** mainstream glibc Linux desktop distributions using Wayland or X11.
- **Compatibility evidence:** current Ubuntu/Debian and Fedora/Kinoite releases in the release test matrix.
- **Non-goals:** musl-based Linux, arbitrary obsolete glibc versions, proprietary GPU-driver coverage without test evidence, or a self-contained Mesa/GPU-driver stack.

AppImage is one distributable file, not a complete operating system. It continues to rely on the host kernel, compositor, graphics driver, Mesa/EGL implementation, and desktop session.

## Packaging policy

Fini uses a minimal, pinned Fini-maintained patch applied in CI to an exact upstream Tauri source revision until an equivalent upstream Tauri release is adopted.

The patch must:

1. Change only AppImage dependency selection and generated launcher behavior necessary for portable desktop ABI compatibility.
2. Preserve normal Tauri application and WebKit helper lookup.
3. Avoid bundling host-coupled low-level runtime libraries identified by the matching upstream Tauri issue.
4. Avoid disabling host GStreamer plugin discovery when media-framework bundling is disabled.
5. Be pinned to an upstream Tauri revision, carry a link to the upstream issue, and have an explicit removal/adoption condition.

The release pipeline must invoke the patched bundler directly. It must not mutate an already-created release AppImage afterward.

## Acceptance criteria

1. The generated AppImage is produced through the pinned custom bundler path.
2. Extracted-artifact checks prove the patched bundler emitted the intended dependency layout.
3. The AppImage opens a visible Fini window on each agreed Wayland/X11 matrix target.
4. App startup has no WebKit helper abort and no EGL initialization failure in the supported matrix.
5. The CI result preserves release artifact and signing behavior.
6. An upstream Tauri upgrade removes the local patch only after the same matrix passes.

## References

- Tauri AppImage documentation: https://v2.tauri.app/distribute/appimage/
- Tauri upstream packaging regression: https://github.com/tauri-apps/tauri/issues/15665
- Tauri experimental portable-AppImage work, not yet adopted: https://github.com/tauri-apps/tauri/pull/12491
