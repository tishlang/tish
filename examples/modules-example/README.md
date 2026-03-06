# Modules Example

Demonstrates Tish's import/export syntax for splitting code across multiple files.

## Layout

```
src/
├── main.tish    # Entry point, imports greet from greet.tish
└── greet.tish   # Exports the greet function
```

## Run

```bash
tish run src/main.tish
```

## Compile

```bash
tish compile src/main.tish -o hello
./hello
```

Output: `Hello, World`
