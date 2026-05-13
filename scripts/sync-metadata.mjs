import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

const paths = {
  packageJson: path.join(rootDir, 'package.json'),
  tauriConfig: path.join(rootDir, 'src-tauri', 'tauri.conf.json'),
  cargoToml: path.join(rootDir, 'src-tauri', 'Cargo.toml')
};

const ensureString = (value, field) => {
  if (!value || typeof value !== 'string') {
    throw new Error(`Missing required string field: ${field}`);
  }
};

const packageJson = JSON.parse(await readFile(paths.packageJson, 'utf8'));
ensureString(packageJson.version, 'package.json.version');
ensureString(packageJson.description, 'package.json.description');
ensureString(packageJson.license, 'package.json.license');
ensureString(packageJson.repository, 'package.json.repository');
ensureString(packageJson.homepage, 'package.json.homepage');

const metadata = packageJson.metadata ?? {};
ensureString(metadata.productName, 'package.json.metadata.productName');
ensureString(metadata.identifier, 'package.json.metadata.identifier');
if (!Array.isArray(metadata.authors) || metadata.authors.some((item) => typeof item !== 'string')) {
  throw new Error('Missing required string array field: package.json.metadata.authors');
}

const updateJsonStringField = (text, key, value) => {
  const pattern = new RegExp(`("${key}"\\s*:\\s*)"[^"]*"`);
  if (!pattern.test(text)) {
    throw new Error(`Failed to find JSON key: ${key}`);
  }
  return text.replace(pattern, `$1"${value}"`);
};

let tauriText = await readFile(paths.tauriConfig, 'utf8');
const tauriConfig = JSON.parse(tauriText);

if (Object.prototype.hasOwnProperty.call(tauriConfig, 'version')) {
  tauriText = updateJsonStringField(tauriText, 'version', packageJson.version);
} else if (tauriConfig.package && Object.prototype.hasOwnProperty.call(tauriConfig.package, 'version')) {
  tauriText = tauriText.replace(/("package"\s*:\s*\{[\s\S]*?"version"\s*:\s*)"[^"]*"/, `$1"${packageJson.version}"`);
} else {
  throw new Error('Failed to find version field in tauri.conf.json');
}

if (tauriConfig.package && Object.prototype.hasOwnProperty.call(tauriConfig.package, 'productName')) {
  tauriText = tauriText.replace(/("package"\s*:\s*\{[\s\S]*?"productName"\s*:\s*)"[^"]*"/, `$1"${metadata.productName}"`);
}
if (tauriConfig.tauri?.bundle && Object.prototype.hasOwnProperty.call(tauriConfig.tauri.bundle, 'identifier')) {
  tauriText = tauriText.replace(/("bundle"\s*:\s*\{[\s\S]*?"identifier"\s*:\s*)"[^"]*"/, `$1"${metadata.identifier}"`);
}
if (tauriConfig.tauri?.bundle && Object.prototype.hasOwnProperty.call(tauriConfig.tauri.bundle, 'shortDescription')) {
  tauriText = tauriText.replace(
    /("bundle"\s*:\s*\{[\s\S]*?"shortDescription"\s*:\s*)"[^"]*"/,
    `$1"${packageJson.description}"`
  );
}
if (tauriConfig.tauri?.bundle && Object.prototype.hasOwnProperty.call(tauriConfig.tauri.bundle, 'longDescription')) {
  tauriText = tauriText.replace(
    /("bundle"\s*:\s*\{[\s\S]*?"longDescription"\s*:\s*)"[^"]*"/,
    `$1"${packageJson.description}"`
  );
}

await writeFile(paths.tauriConfig, tauriText, 'utf8');

const replaceInPackageSection = (text, key, rawValue) => {
  const lines = text.split(/\r?\n/);
  let inPackage = false;
  let replaced = false;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const sectionMatch = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (sectionMatch) {
      inPackage = sectionMatch[1].trim() === 'package';
      continue;
    }

    if (inPackage && new RegExp(`^\\s*${key}\\s*=`).test(line)) {
      lines[i] = `${key} = ${rawValue}`;
      replaced = true;
      break;
    }
  }

  if (!replaced) {
    throw new Error(`Failed to update [package].${key} in Cargo.toml`);
  }

  return lines.join('\n');
};

let cargoText = await readFile(paths.cargoToml, 'utf8');
cargoText = replaceInPackageSection(cargoText, 'version', `"${packageJson.version}"`);
cargoText = replaceInPackageSection(cargoText, 'description', `"${packageJson.description}"`);
cargoText = replaceInPackageSection(cargoText, 'license', `"${packageJson.license}"`);
cargoText = replaceInPackageSection(cargoText, 'repository', `"${packageJson.repository}"`);
cargoText = replaceInPackageSection(cargoText, 'authors', `[${metadata.authors.map((name) => `"${name}"`).join(', ')}]`);

await writeFile(paths.cargoToml, `${cargoText.endsWith('\n') ? cargoText : `${cargoText}\n`}`, 'utf8');

console.log(`Synchronized package metadata (${packageJson.version}) to tauri.conf.json and Cargo.toml.`);
