# COSMIC panel applet

Native COSMIC 1.8 frontend for the existing `ai-usage.service`. Displays remaining quota percentages or balances on the panel and all quota windows, reset times and errors in its popup. Account email addresses are masked; click an account header to reveal one until the popup closes. The popup anchors to the actual panel button bounds, has a 480 px height cap, and uses a 4 px scroll thumb within an 8 px track. `~` marks stale data. No additional network requests or credentials.

From this directory:

```sh
unset APPIMAGE APPDIR LD_LIBRARY_PATH
cargo test --locked --release --jobs 2
cargo build --locked --release --jobs 2
install -Dm755 target/release/ai-usage-cosmic ~/.local/bin/ai-usage-cosmic
install -Dm644 io.github.netherguy4.AiUsage.desktop ~/.local/share/applications/io.github.netherguy4.AiUsage.desktop
```

Add **AI Usage** in COSMIC panel settings. Keep `ai-usage.service` running. The applet reads `$XDG_RUNTIME_DIR/ai-usage/state.json` every five seconds and shares the backend's schema and provider icons. Rust comes from a local `rustup override`, the C toolchain is the layered system `gcc`; the base OS needs no compiler installation.

`Cargo.lock` pins libcosmic and its configuration crate together to COSMIC 1.8 (`d4d71fd53e5ed6bd3a430089114dffa2da3cd498`).
