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

Launching from a terminal returns the prompt immediately: the GUI detaches into
its own background session. Pass `--foreground` to keep it attached (useful
under systemd or when debugging).

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
          "remote_control": true,
          "env_vars": [
            { "key": "NODE_ENV", "value": "production" },
            { "key": "DB_HOST",  "value": "localhost" }
          ]
        }
      ]
    }
  ],
  "control": {
    "enabled": true,
    "port": 0,
    "allowed_actions": []
  }
}
```

Log path placeholders:
- `{name}` → component display name
- `{date}` → `YYYY-MM-DD`

---

## Remote control (CLI, scripts, agents)

A running instance can be driven from outside — e.g. an AI agent that just
rebuilt your MCP server and wants it restarted. The same binary doubles as the
client; point it at the same config file:

```bash
# Stop → wait for exit → start again. Blocks until the process is back up.
./proconductor production.json --restart "MCP Server"

./proconductor production.json --start  "API Server"
./proconductor production.json --stop   "Workers"        # group name works too
./proconductor production.json --restart all
./proconductor production.json --status                  # JSON: state, pid, uptime, last exit code
./proconductor production.json --restart api --timeout 60
```

Targets are matched by component name or id, group name or id, or `all`
(names case-insensitive). Exit code is `0` on success, `1` on failure or
timeout (default 30 s); the instance's reply is printed to stdout/stderr, so
an agent can simply shell out and check the exit code.

### Under the hood — control port

The CLI is a thin client over a loopback TCP socket, so anything that can open
a socket can control ProConductor (cron, systemd hooks, CI, Task Scheduler,
any language) — event-driven, no polling, identical on Windows, macOS and
Linux. The instance listens on `127.0.0.1` and publishes the port in
`production.json.port` next to the config:

```bash
printf 'restart MCP Server\n' | nc 127.0.0.1 $(cat production.json.port)
# → {"ok":true,"message":"done"}
```

Protocol: one request line, one reply line. The request is either plain text
`restart MCP Server` or JSON `{"action":"restart","target":"MCP Server"}`;
the reply is `{"ok": true|false, "message": "..."}`. For `stop`/`restart` the
reply arrives only once the process is really down / really running again.

This works while the window is minimized or hidden in the tray.

### Configuration

Top-level `"control"` block in the config file:

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Master switch. `false` = nothing listens, CLI reports the instance as unreachable. |
| `port` | `0` | Loopback TCP port. `0` = OS-assigned; the actual port is always written to `<config>.port`. Set a fixed port if clients should not have to read the file. |
| `allowed_actions` | `[]` (all) | Subset of `start`, `stop`, `restart`, `status` that clients may issue. |

Per component, `"remote_control": false` (also a checkbox in the Configure
view) excludes it from remote start/stop/restart — a group- or `all`-wide
command silently skips it, a direct command is refused.

The socket is bound to loopback only, so any local user can talk to it —
the same trust boundary as the config file and the processes themselves.

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

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
