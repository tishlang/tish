# Minimal Rust runtime container (~20MB base)
# Mount binaries at runtime - no binary embedded by default
#
# Usage:
#   docker build -t tish-runtime .
#   docker run -v /path/to/binary:/app/binary:ro tish-runtime
#
# To embed a binary, use Dockerfile.embedded instead
#
# Note: Uses distroless/cc for glibc support (required for standard Rust binaries)
# For static musl binaries, use distroless/static instead
# Build Linux binaries on Debian 12-era glibc (e.g. Dockerfile.compile rust:bookworm) so symbols
# match this image; building on newer distros can require GLIBC_2.38+ and fail here.

FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

# Run as non-root user (uid 65532, provided by distroless:nonroot)
USER nonroot:nonroot

ENTRYPOINT ["/app/binary"]
