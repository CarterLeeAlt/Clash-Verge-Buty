import fs from "fs-extra";
import path from "path";
import AdmZip from "adm-zip";
import { createRequire } from "module";
import { getOctokit, context } from "@actions/github";

const target = process.argv.slice(2)[0];
const alpha = process.argv.slice(2)[1];

const ARCH_MAP = {
  "x86_64-pc-windows-msvc": "x64",
  "aarch64-pc-windows-msvc": "arm64",
};

async function getTauriPackageInfo() {
  const tauriConfigPath = path.resolve("./src-tauri/tauri.conf.json");
  const tauriConfig = await fs.readJson(tauriConfigPath);

  const productName = tauriConfig?.package?.productName;
  const version = tauriConfig?.package?.version;

  if (!productName) {
    throw new Error(
      "package.productName not found in src-tauri/tauri.conf.json"
    );
  }

  if (!version) {
    throw new Error("package.version not found in src-tauri/tauri.conf.json");
  }

  return { productName, version };
}

/// Script for ci
/// 打包绿色版/便携版 (only Windows)
async function resolvePortable() {
  if (process.platform !== "win32") return;

  const { productName, version } = await getTauriPackageInfo();
  const productFileName = productName.replace(/ /g, ".");

  const releaseDir = target
    ? `./src-tauri/target/${target}/release`
    : `./src-tauri/target/release`;
  const configDir = path.join(releaseDir, ".config");

  if (!(await fs.pathExists(releaseDir))) {
    throw new Error("could not found the release dir");
  }

  await fs.mkdirp(configDir);
  await fs.createFile(path.join(configDir, "PORTABLE"));

  const exePath = path.join(releaseDir, `${productName}.exe`);
  if (!(await fs.pathExists(exePath))) {
    throw new Error(`File not found: ${exePath}`);
  }

  const clashMetaPath = path.join(releaseDir, "clash-meta.exe");
  if (!(await fs.pathExists(clashMetaPath))) {
    throw new Error(`File not found: ${clashMetaPath}`);
  }

  const clashMetaAlphaPath = path.join(releaseDir, "clash-meta-alpha.exe");
  if (!(await fs.pathExists(clashMetaAlphaPath))) {
    throw new Error(`File not found: ${clashMetaAlphaPath}`);
  }

  const resourcesPath = path.join(releaseDir, "resources");
  if (!(await fs.pathExists(resourcesPath))) {
    throw new Error(`Folder not found: ${resourcesPath}`);
  }

  const zip = new AdmZip();

  zip.addLocalFile(exePath);
  zip.addLocalFile(clashMetaPath);
  zip.addLocalFile(clashMetaAlphaPath);
  zip.addLocalFolder(resourcesPath, "resources");
  zip.addLocalFolder(configDir, ".config");

  const zipFile = `${productFileName}_${version}_${ARCH_MAP[target]}_portable.zip`;
  zip.writeZip(zipFile);

  console.log("[INFO]: create portable zip successfully");

  // push release assets
  if (process.env.GITHUB_TOKEN === undefined) {
    throw new Error("GITHUB_TOKEN is required");
  }

  const options = { owner: context.repo.owner, repo: context.repo.repo };
  const github = getOctokit(process.env.GITHUB_TOKEN);
  const tag = alpha ? "alpha" : process.env.TAG_NAME || `v${version}`;
  console.log("[INFO]: upload to ", tag);

  const { data: release } = await github.rest.repos.getReleaseByTag({
    ...options,
    tag,
  });

  const assets = release.assets.filter((x) => x.name === zipFile);
  if (assets.length > 0) {
    const id = assets[0].id;
    await github.rest.repos.deleteReleaseAsset({
      ...options,
      asset_id: id,
    });
  }

  console.log(release.name);

  await github.rest.repos.uploadReleaseAsset({
    ...options,
    release_id: release.id,
    name: zipFile,
    data: zip.toBuffer(),
  });
}

resolvePortable().catch(console.error);
