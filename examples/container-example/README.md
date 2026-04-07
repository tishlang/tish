# Container Example

**Docker** workflows for **Tish** programs: compile `.tish` to **Linux x86_64** binaries with **glibc**, run them in **distroless**-sized images, and optionally deploy to **[Zectre](https://zectre.com)** as a container app.

## Prerequisites

- **Docker** (Desktop or Engine)
- **[just](https://github.com/casey/just)** (`cargo install just` or `brew install just`)
- **Tish language repo on disk** (default **`../tish`**, sibling of `tish-paas`) — used only as the **Docker build context** for **`Dockerfile.compile`**. The image is built **inside** Docker (`cargo build` there); you do **not** need a **`tish` binary on your PATH** for `just setup`, `just build`, `just zectre-build-image`, etc.
- For Zectre: **[zectre](https://github.com/tishlang/zectre-cli)** CLI and a cluster with **container** support enabled on agents

### When you *do* need Rust / a local compiler

Recipes like **`just build-local`** run **`cargo run -p tish …`** in the Tish repo on your machine. Those need **Rust** and the **`../tish`** checkout, still not necessarily a global `tish` install (Cargo runs the crate). Use **`just build`** if you want zero host toolchain beyond Docker.

## Quick start

```bash
# One-time: runtime image (distroless) + compiler image (tish CLI inside Docker)
just setup

# Compile a program to a Linux binary (uses compiler container; default platform linux/amd64)
just build sample.tish ./sample-bin

# Run it in the minimal runtime (binary mounted read-only)
just docker-run ./sample-bin

# HTTP example (src/main.tish): publish host 8080 → container 8080 and set PORT
just build src/main.tish ./server
DOCKER_PORT=8080 just docker-run ./server

# Or one-shot recipes
just example        # CLI sample
just example-http   # build + run HTTP with port 8080
```

See `just --list` for all recipes. The `justfile` header comments mirror this flow.

## Which Dockerfile is which?

| File | Role | Build context | Used by `just`? |
|------|------|---------------|-----------------|
| **`Dockerfile.compile`** | Image containing the **Tish compiler** (`tish build`). **Debian bookworm** (glibc) so linked artifacts match **distroless/cc-debian12**. | **`../tish`** (the language repo root, not this folder) | **`build-compiler`**, then **`build`** |
| **`Dockerfile`** | **Runtime only**: `gcr.io/distroless/cc-debian12`, entrypoint runs **`/app/binary`**. Binary is **mounted** at run time. | This repo (`.`) | **`build-runtime`**, **`docker-run`** |
| **`Dockerfile.embedded`** | Same distroless/cc runtime, but **copies** a pre-built binary into the image (no mount). Good for **Zectre** / registries. | This repo (`.`) | **`docker-build-embedded`**, **`zectre-build-image`** |

**Flow**

- **`Dockerfile.compile`** → image that **is** the `tish` tool: compile `.tish` → Linux **glibc** binaries inside Docker.
- **`Dockerfile`** / **`Dockerfile.embedded`** → minimal **runtime** for those binaries on **distroless/cc** (mount vs bake-in).

## Layout

- **`src/console.tish`** — tiny CLI smoke test  
- **`src/main.tish`** — HTTP server (`serve`, `/`, `/health`); uses `PORT` from the environment (default `8080`)  
- **`zectre.yaml`** — Zectre manifest (`process_type: docker`, image name, networking)

## Zectre deployments

Zectre agents run **`docker pull`** only if the image is **not** already present locally. A short name like `zectre-minimal-container:latest` is **not** on Docker Hub unless you publish it—see **`zectre.yaml`** comments and [Container deployments](https://github.com/tishlang/tish/tree/main/examples/container-example).

**Local / same host as the agent**

```bash
just zectre-build-image          # build binary + embedded image with tag from ZECTRE_IMAGE
just zectre-deploy --wait        # uses ZECTRE_API_URL (default http://localhost:47080)
```

**Remote agents**

Build, tag, **push** to GHCR/Docker Hub, set `deploy.image` in `zectre.yaml` to the full reference (same idea as [container-example](https://github.com/tishlang/tish/tree/main/examples/container-example)).

## Environment variables (common)

| Variable | Default | Purpose |
|----------|---------|---------|
| `DOCKER_PLATFORM` | `linux/amd64` | `docker build` / `docker run` platform |
| `DOCKER_PORT` | (unset) | If set, `docker-run*` adds `-p` and `-e PORT=…` |
| `DOCKER_PORT_HOST` | same as `DOCKER_PORT` | Host side of the port mapping |
| `ZECTRE_IMAGE` | `zectre-minimal-container:latest` | Tag for embedded Zectre image |
| `ZECTRE_API_URL` | `http://localhost:47080` | Passed to `zectre deploy` |

## Language reference

Tish syntax and APIs: [LANGUAGE.md](https://github.com/tishlang/tish/blob/main/docs/LANGUAGE.md) in the `tish` repo.
