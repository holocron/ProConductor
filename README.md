# ProConductor

A standalone, single-binary GUI orchestrator for multi-component applications.
Built with **Rust + egui** — no runtime, no WebView, no installer needed.

---

## Usage

```bash
# Default — reads/writes proconductor.json in current directory
./proconductor

# Explicit config — run multiple instances with different configs
./proconductor production.json
./proconductor staging.json
./proconductor my-app/config.json
```

The config file is created automatically if it doesn't exist.
Each invocation is a fully independent instance.

---

## Config file format

The config is plain JSON, human-readable and version-control friendly:

```json
{
  "groups": [
    {
      "id": "uuid-...",
      "name": "My Microservice App",
      "components": [
        {
          "id": "uuid-...",
          "name": "API Server",
          "executable": "/usr/bin/node",
          "working_dir": "/opt/myapp/api",
          "args": "server.js --port 8080",
          "log_path": "/var/log/myapp/{name}-{date}.log",
          "run_as_user": "",
          "env_vars": [
            { "key": "NODE_ENV", "value": "production" },
            { "key": "DB_HOST",  "value": "localhost" }
          ]
        }
      ]
    }
  ]
}
```

Log path placeholders:
- `{name}` → component display name
- `{date}` → `YYYY-MM-DD`

---

## Build

### Requirements

| Platform | Requirement |
|---|---|
| **All** | Rust 1.70+ (`rustup update stable`) |
| **Linux** | `sudo apt install libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev` |
| **macOS** | Xcode Command Line Tools (`xcode-select --install`) |
| **Windows** | MSVC build tools (Visual Studio installer) |

### Build commands

```bash
# Debug (fast compile, larger binary)
cargo run

# Release (optimized, stripped, ~8–15MB)
cargo build --release

# Output:
#   Linux/macOS → target/release/proconductor
#   Windows     → target/release/proconductor.exe
```

The release binary is fully self-contained — copy it anywhere and run it.

### Cross-compilation

```bash
# Linux → Windows (from Linux with mingw)
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu

# macOS ARM + Intel universal binary
rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
lipo -create -output proconductor \
  target/x86_64-apple-darwin/release/proconductor \
  target/aarch64-apple-darwin/release/proconductor
```

For CI, use the [cross](https://github.com/cross-rs/cross) tool.

---

## Run as User (Unix)

If "Run as User" is set, ProConductor prepends `sudo -u <user>` to the command.
The user running ProConductor must have passwordless sudo for this to work:

```
# /etc/sudoers.d/proconductor
myuser ALL=(www-data) NOPASSWD: ALL
```

---

## Multiple simultaneous instances

Since each instance reads/writes its own config file, you can run as many as
you want — each with a different `.json` file and its own window:

```bash
./proconductor backend.json &
./proconductor frontend.json &
./proconductor workers.json &
```

---

## Distribution

The single `proconductor` binary is all you need to ship.
No installer, no dependencies, no runtime — just copy and run.

For desktop integration (optional):
- **Linux**: place a `.desktop` file in `~/.local/share/applications/`
- **macOS**: wrap in a `.app` bundle with `cargo-bundle`
- **Windows**: create a shortcut with the desired `.json` as argument
