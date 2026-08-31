# Linux & Docker Guide

Nexus provides first-class support for Linux in headless environments, CI pipelines, and containerized AI agent workflows.

---

## 1. Plugin Search Paths

On Linux, Nexus automatically scans standard user and system directories for CLAP and VST3 plugin bundles in order of specificity:

### CLAP Plugins (`.clap`)
- **User scope**: `~/.clap`
- **System scope**: `/usr/lib/clap`, `/usr/local/lib/clap`

### VST3 Plugins (`.vst3` or `.so`)
- **User scope**: `~/.vst3`
- **System scope**: `/usr/lib/vst3`, `/usr/local/lib/vst3`

### Custom Plugin Paths
You can add custom scan directories in `~/.config/nexus/config.toml` (or `~/.nexus.toml`):

```toml
[plugins]
clap = [
    "~/my-plugins/clap",
    "/opt/audio/clap"
]
vst3 = [
    "~/my-plugins/vst3",
    "/opt/audio/vst3"
]
```

---

## 2. Docker & Docker Compose

Nexus includes a multi-stage `Dockerfile` and `docker-compose.yml` for automated testing and running the headless `nexus-mcp` server.

### Starting the MCP Server

To build and start `nexus-mcp` on port `9997` (Streamable HTTP):

```bash
docker compose up nexus-mcp
```

Test connectivity:
```bash
curl http://127.0.0.1:9997/mcp
```

### Running Tests in Docker

To execute the entire workspace test suite inside a Linux container:

```bash
docker compose run --rm test
```

### Mounting Plugins into Containers

Mount your host plugin directories into the container's standard search paths in `docker-compose.yml`:

```yaml
volumes:
  - ./plugins/clap:/root/.clap:ro
  - ./plugins/vst3:/root/.vst3:ro
```

---

## 3. Native Linux Build & Run

### Prerequisites (Debian/Ubuntu)
```bash
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libasound2-dev
```

### Building `nexus-mcp`
```bash
cargo build --release -p nexus-mcp
```

### Running `nexus-mcp`
- **Streamable HTTP (default port 9997)**:
  ```bash
  ./target/release/nexus-mcp --port 9997
  ```
- **Stdio transport (for IDEs / desktop agents)**:
  ```bash
  ./target/release/nexus-mcp --stdio
  ```

---

## 4. Headless & Driver-Free Architecture

- **No Hardware Audio Driver Required**: Offline sampling (`render_offline`) and tool execution render pure 64-bit/32-bit floating-point audio in memory without needing ALSA, PulseAudio, or PipeWire daemon configuration.
- **No Display Server Required**: The standalone MCP server runs completely headless without X11 or Wayland dependencies.
- **Dynamic Module Loading**: Linux VST3 plugins (`.so` binaries under `Contents/x86_64-linux/` or bundle root) are loaded using standard POSIX `dlopen`/`dlsym` dynamic linking.
