# Hello World

The simplest possible Tish application. Logs output and exits.

## Features Used

None (runs in secure mode).

## What It Does

- Logs a greeting message
- Logs the version
- Exits successfully

## Local Development

```bash
# Run with interpreter
tish run src/main.tish

# Compile and run
tish compile src/main.tish -o hello
./hello
```

## Deploy to Tish Platform

```bash
tish-cli login
tish-cli projects create hello-world
tish-cli link
tish-cli deploy --wait
```
