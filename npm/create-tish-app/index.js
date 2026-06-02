#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');
const readline = require('readline');

function prompt(question) {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise(resolve => {
    rl.question(question, answer => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

function copyDirSync(src, dest, safeName) {
  fs.mkdirSync(dest, { recursive: true });
  let entries = fs.readdirSync(src, { withFileTypes: true });

  for (let entry of entries) {
    let srcPath = path.join(src, entry.name);
    let destPath = path.join(dest, entry.name);

    if (entry.isDirectory()) {
      copyDirSync(srcPath, destPath, safeName);
    } else {
      let content = fs.readFileSync(srcPath, 'utf8');
      content = content.replace(/\{\{PROJECT_NAME\}\}/g, safeName);
      fs.writeFileSync(destPath, content, 'utf8');
    }
  }
}

async function main() {
  const templatesDir = path.join(__dirname, 'templates');
  const TEMPLATES = fs.existsSync(templatesDir) ? fs.readdirSync(templatesDir).filter(f => fs.statSync(path.join(templatesDir, f)).isDirectory()) : [];

  let template = (process.argv[2] && String(process.argv[2]).trim()) || '';
  let name = (process.argv[3] && String(process.argv[3]).trim()) || '';

  // Shift arguments if the first arg is not a template (allow `create-tish-app my-app` but prompt for template)
  if (template && !TEMPLATES.includes(template)) {
    if (!name) {
      name = template;
      template = '';
    }
  }

  if (!template) {
    console.log('Available templates:');
    TEMPLATES.forEach((t, i) => console.log(`  ${i + 1}) ${t}`));
    let selection = await prompt('Select a template (1-' + TEMPLATES.length + '): ');
    const index = parseInt(selection, 10) - 1;
    if (index >= 0 && index < TEMPLATES.length) {
      template = TEMPLATES[index];
    } else if (TEMPLATES.includes(selection)) {
      template = selection;
    }
    
    if (!TEMPLATES.includes(template)) {
      console.error('Invalid template selection.');
      process.exit(1);
    }
  }

  if (!name) {
    name = await prompt('Project name: ');
    name = name.trim();
  }
  
  if (!name) {
    console.error('Please provide a project name.');
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

  const safeName = name.replace(/[^a-z0-9-]/gi, '-').replace(/-+/g, '-').toLowerCase() || 'tish-app';

  const tplDir = path.join(templatesDir, template);
  if (!fs.existsSync(tplDir)) {
    console.error('Template not found: ' + template);
    process.exit(1);
  }

  console.log(`Copying files from ${template} template...`);
  copyDirSync(tplDir, dir, safeName);

  console.log(`\nSuccess! Created ${name} at ${dir}`);
  console.log('Inside that directory, you can run several commands:');
  console.log('\n  npm run dev');
  console.log('    Starts the development server.');
  console.log('\n  npm run build');
  console.log('    Builds the app for production.');
  console.log('\n  npm run start');
  console.log('    Runs the built app in production mode.\n');
  console.log('We suggest that you begin by typing:');
  console.log(`\n  cd ${name}`);
  console.log('  npm run dev\n');
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
