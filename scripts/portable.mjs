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

  const releaseExeFiles = (await fs.readdir(releaseDir))
    .filter((name) => name.toLowerCase().endsWith(".exe"))
    .sort();
  const nsisDir = path.join(releaseDir, "bundle", "nsis");
  const nsisExeFiles = (await fs.pathExists(nsisDir))
    ? (await fs.readdir(nsisDir))
        .filter((name) => name.toLowerCase().endsWith(".exe"))
        .sort()
    : [];

  console.log("[INFO]: target/release exe files:", releaseExeFiles);
  console.log("[INFO]: target/release/bundle/nsis exe files:", nsisExeFiles);

  const preferredNames = [
    "Clash_Verge_Buty.exe",
    `${productName}.exe`,
    `${productName.replace(/-/g, "_")}.exe`,
    `${productName.toLowerCase()}.exe`,
    `${productName.toLowerCase().replace(/-/g, "_")}.exe`,
  ];

  const deniedKeyword = /(setup|setup_unsigned|installer)/i;
  const portableExeName =
    preferredNames.find((name) => releaseExeFiles.includes(name)) ||
    releaseExeFiles.find(
      (name) => !deniedKeyword.test(name) && !/clash-meta(-alpha)?\.exe$/i.test(name)
    );

  if (!portableExeName) {
    throw new Error(`Portable main exe not found under ${releaseDir}`);
  }
  if (deniedKeyword.test(portableExeName)) {
    throw new Error(`Portable main exe resolved to installer-like file: ${portableExeName}`);
  }
  const exePath = path.join(releaseDir, portableExeName);

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

  const unsignedSuffix = process.env.UNSIGNED_BUILD === "1" ? "_unsigned" : "";
  const zipFile = `${productFileName}_${version}_${ARCH_MAP[target]}_portable${unsignedSuffix}.zip`;
  zip.writeZip(zipFile);

  const zipCheck = new AdmZip(zipFile)
    .getEntries()
    .map((entry) => entry.entryName);
  console.log("[INFO]: portable zip entries:", zipCheck);

  const zipExeEntries = zipCheck.filter((name) => name.toLowerCase().endsWith(".exe"));
  const zipEntriesLower = zipCheck.map((name) => name.toLowerCase());
  if (!zipExeEntries.some((name) => path.basename(name).toLowerCase() === portableExeName.toLowerCase())) {
    throw new Error(`portable.zip missing main exe: ${portableExeName}`);
  }
  if (!zipExeEntries.some((name) => path.basename(name).toLowerCase() === "clash-meta.exe")) {
    throw new Error("portable.zip missing clash-meta.exe");
  }
  if (!zipExeEntries.some((name) => path.basename(name).toLowerCase() === "clash-meta-alpha.exe")) {
    throw new Error("portable.zip missing clash-meta-alpha.exe");
  }
  if (!zipEntriesLower.some((name) => name.startsWith("resources/"))) {
    throw new Error("portable.zip missing resources/ directory");
  }
  if (!zipEntriesLower.some((name) => name.startsWith(".config/"))) {
    console.warn("[WARN]: portable.zip missing .config/ directory");
  }
  if (zipExeEntries.some((name) => deniedKeyword.test(path.basename(name)))) {
    throw new Error(`portable.zip contains installer exe: ${zipExeEntries.join(", ")}`);
  }

  console.log("[INFO]: create portable zip successfully");

  // push release assets
  if (process.env.SKIP_RELEASE_UPLOAD === "1") {
    console.log("[INFO]: skip release upload by SKIP_RELEASE_UPLOAD=1");
    return;
  }

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
