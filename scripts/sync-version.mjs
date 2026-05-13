import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

const packageJsonPath = path.join(rootDir, 'package.json');
const tauriConfigPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const cargoTomlPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');

const packageJsonText = await readFile(packageJsonPath, 'utf8');
const packageJson = JSON.parse(packageJsonText);

if (!packageJson.version || typeof packageJson.version !== 'string') {
  throw new Error('Missing required "version" field in package.json');
}

const version = packageJson.version;

const tauriConfigText = await readFile(tauriConfigPath, 'utf8');
const tauriConfig = JSON.parse(tauriConfigText);

if (Object.prototype.hasOwnProperty.call(tauriConfig, 'version')) {
  tauriConfig.version = version;
} else {
  tauriConfig.package ??= {};
  tauriConfig.package.version = version;
}

await writeFile(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`, 'utf8');

const cargoTomlText = await readFile(cargoTomlPath, 'utf8');
const cargoLines = cargoTomlText.split(/\r?\n/);
let inPackageSection = false;
let packageVersionUpdated = false;

for (let i = 0; i < cargoLines.length; i += 1) {
  const line = cargoLines[i];
  const sectionMatch = line.match(/^\s*\[([^\]]+)\]\s*$/);

  if (sectionMatch) {
    inPackageSection = sectionMatch[1].trim() === 'package';
    continue;
  }

  if (inPackageSection && /^\s*version\s*=\s*/.test(line)) {
    cargoLines[i] = line.replace(/^(\s*version\s*=\s*)["'][^"']*["'](\s*(?:#.*)?)$/, `$1"${version}"$2`);
    packageVersionUpdated = true;
    break;
  }
}

if (!packageVersionUpdated) {
  throw new Error('Failed to find [package].version in src-tauri/Cargo.toml');
}

await writeFile(cargoTomlPath, `${cargoLines.join('\n')}\n`, 'utf8');

console.log(`Synchronized version to ${version} in tauri.conf.json and Cargo.toml.`);
