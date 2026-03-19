#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');

const projectName = process.argv[2] || getProjectNameFromCwd();

function getProjectNameFromCwd() {
  const cwd = process.cwd();
  const base = path.basename(cwd);
  if (base && base !== '.' && !fs.existsSync(path.join(cwd, 'src'))) {
    return base;
  }
  return null;
}

function prompt(question) {
  const readline = require('readline');
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise(resolve => {
    rl.question(question, answer => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

async function main() {
  let name = projectName;
  if (!name) {
    name = await prompt('Project name: ');
    if (!name) {
      console.error('Please provide a project name: npx @tishlang/create-tish-app my-app');
      process.exit(1);
    }
  }

  const dir = path.resolve(process.cwd(), name);
  if (fs.existsSync(dir)) {
    console.error(`Directory already exists: ${dir}`);
    process.exit(1);
  }

  fs.mkdirSync(dir, { recursive: true });
  fs.mkdirSync(path.join(dir, 'src'), { recursive: true });

  const safeName = name.replace(/[^a-z0-9-]/gi, '-').replace(/-+/g, '-').toLowerCase() || 'tish-app';
  const files = {
    'src/main.tish': `// ${name} - Tish app
let message = "Hello, Tish!"
console.log(message)
`,
    'tish.yaml': `name: ${safeName}
`,
    'package.json': JSON.stringify({
      name: safeName,
      version: '0.1.0',
      private: true,
      tish: { source: './src/main.tish' },
    }, null, 2),
    '.gitignore': `# Build output
/tish_out
*.exe
`,
    'README.md': `# ${name}

A [Tish](https://github.com/tishlang/tish) project.

## Run (interpret)

\`\`\`bash
npx @tishlang/tish run src/main.tish
# or after installing: tish run src/main.tish
\`\`\`

## Compile to native

\`\`\`bash
npx @tishlang/tish compile src/main.tish -o app
./app
\`\`\`
`,
  };

  for (const [file, content] of Object.entries(files)) {
    fs.writeFileSync(path.join(dir, file), content, 'utf8');
  }

  console.log(`Created ${name} at ${dir}`);
  console.log('');
  console.log('Next steps:');
  console.log(`  cd ${name}`);
  console.log('  npx @tishlang/tish run src/main.tish');
  console.log('  # or: npx @tishlang/tish compile src/main.tish -o app && ./app');
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
