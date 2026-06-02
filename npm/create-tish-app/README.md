# @tishlang/create-tish-app

The official scaffolding tool for Tish projects. It provides templates to quickly bootstrap new applications with zero configuration.

## Usage

You can create a new project by running:

```bash
npx @tishlang/create-tish-app [template] [project-name]
```

If you run it without arguments, it will launch an interactive prompt to help you choose a template and name your project:

```bash
npx @tishlang/create-tish-app
```

### Available Templates

- **`lattish`**: A client-side SPA with `@tishlang/lattish` and a Tish static file server.
- **`http-hello`**: A simple standalone HTTP server written in Tish.
- **`docs`**: A static site generator for Markdown documentation.

## Local Development & Debugging

If you are modifying the scaffolding tool or adding new templates in the `templates/` directory, you can easily test it locally.

### Method 1: Direct Execution
Run the `index.js` script directly using Node.js in a scratch directory:
```bash
mkdir -p /tmp/test-app && cd /tmp/test-app
node ~/Projects/tish/tish/npm/create-tish-app/index.js
```

### Method 2: Using `npm link`
You can link the package globally to test the CLI command exactly as an end-user would:
```bash
# From this directory:
npm link

# Now you can use it anywhere on your machine
cd /tmp
create-tish-app docs my-test-docs
```
