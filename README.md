# Wrapped Texture Editor

A Rust desktop app (egui + wgpu) for painting textures directly on 3D models: brushes, layers, image and text
stamps (projected, UV, or wrapped on the surface), PNG / PSD / ORA export and `.wtep` projects.

- Build: `cargo build --release -p wte_app` → `target/release/wrapped_texture_editor.exe`
- Test: `cargo test --workspace`; headless scenarios: `wrapped_texture_editor.exe [MODEL] --screenshot out.png --scenario <name>`
- Design contract: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- State, decisions, open issues and the checklist for picking the project up: [`HANDOFF.md`](HANDOFF.md)
