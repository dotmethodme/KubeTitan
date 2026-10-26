# KubeTitan

A native desktop IDE for Kubernetes. Rust and egui, no webview: the app talks
straight to your cluster's API server and keeps everything on your machine.

- **Resources, logs, terminal, port forwards** in one window.
- **Port forwards are saved** with their local port and reopen in one click after
  a restart or a context switch.
- **A real terminal emulator** for `exec` sessions: prompts, colours, full-screen
  apps, correct resizes.
- **Metrics beside the logs**: CPU, memory and disk per pod, charted from the
  Prometheus already in your cluster.
- **Secrets and ConfigMaps** as plain key/value, with a diff before anything is
  written.
- **Init containers listed** next to the app containers, and pickable as a log
  target, because a stuck init container is exactly the one you need logs from.
- **Keyboard first**: `Ctrl K` jumps anywhere, `Ctrl .` switches namespace,
  `Ctrl ,` switches context, `Ctrl 1`–`9` switch view.

## Build from source

Needs a Rust toolchain (1.90+) and a kubeconfig that already works with `kubectl`.
On Linux you also need the usual GUI headers (`libxkbcommon-dev libx11-dev
libgl1-mesa-dev libwayland-dev pkg-config`).

```sh
git clone https://github.com/dotmethodme/KubeTitan
cd KubeTitan
cargo run --release
```

Checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. CI runs all three plus a release build.

## Licence

[Apache License 2.0](LICENSE). No account, no telemetry, no server of ours in the
loop. The website is [kubetitan.com](https://kubetitan.com).

Issues and pull requests belong here.
