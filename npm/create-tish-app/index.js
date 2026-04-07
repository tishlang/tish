#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');

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
  let name = (process.argv[2] && String(process.argv[2]).trim()) || '';
  if (!name) {
    name = await prompt('Project name: ');
    name = name.trim();
  }
  if (!name) {
    console.error('Please provide a project name (argument or when prompted). Example: npx @tishlang/create-tish-app my-app');
    process.exit(1);
  }

  if (name.includes(path.sep) || name.includes('/') || name.includes('\\')) {
    console.error('Project name must be a single folder name, not a path.');
    process.exit(1);
  }
  if (name === '.' || name === '..') {
    console.error('Invalid project name.');
    process.exit(1);
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
    'zectre.yaml': `name: ${safeName}
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

## Build to native

\`\`\`bash
npx @tishlang/tish build src/main.tish -o app
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
  console.log('  # or: npx @tishlang/tish build src/main.tish -o app && ./app');
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
