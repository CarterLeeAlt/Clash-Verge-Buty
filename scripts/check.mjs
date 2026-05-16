import fs from "fs-extra";
import zlib from "zlib";
import tar from "tar";
import path from "path";
import AdmZip from "adm-zip";
import fetch from "node-fetch";
import proxyAgent from "https-proxy-agent";
import { execSync } from "child_process";

const cwd = process.cwd();
const TEMP_DIR = path.join(cwd, "node_modules/.verge");
const FORCE = process.argv.includes("--force");

const PLATFORM_MAP = {
  "x86_64-pc-windows-msvc": "win32",
};
const ARCH_MAP = {
  "x86_64-pc-windows-msvc": "x64",
};

const arg1 = process.argv.slice(2)[0];
const arg2 = process.argv.slice(2)[1];
const target = arg1 === "--force" ? arg2 : arg1;
const { platform, arch } = target
  ? { platform: PLATFORM_MAP[target], arch: ARCH_MAP[target] }
  : process;

const SIDECAR_HOST = target
  ? target
  : execSync("rustc -vV")
      .toString()
      .match(/(?<=host: ).+(?=\s*)/g)[0];

const DOWNLOAD_SOURCES = {
  mihomoAlphaVersion: "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha/version.txt",
  mihomoAlphaPrefix: "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha",
  mihomoStableVersion: "https://github.com/MetaCubeX/mihomo/releases/latest/download/version.txt",
  mihomoStablePrefix: "https://github.com/MetaCubeX/mihomo/releases/download",
  simpleScZip:
    "https://nsis.sourceforge.io/mediawiki/images/e/ef/NSIS_Simple_Service_Plugin_Unicode_1.30.zip",
  countryMmdb:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
  geositeDat:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
  geoipDat:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
  enableLoopback:
    "https://github.com/Kuingsmile/uwp-tool/releases/download/latest/enableLoopback.exe",
};

/* ======= clash meta alpha======= */
const META_ALPHA_VERSION_URL = DOWNLOAD_SOURCES.mihomoAlphaVersion;
const META_ALPHA_URL_PREFIX = DOWNLOAD_SOURCES.mihomoAlphaPrefix;
// Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
let META_ALPHA_VERSION;

const META_ALPHA_MAP = {
  "win32-x64": "mihomo-windows-amd64-compatible",
};

// Fetch the latest alpha release version from the version.txt file
async function getLatestAlphaVersion() {
  const options = {};

  const httpProxy =
    process.env.HTTP_PROXY ||
    process.env.http_proxy ||
    process.env.HTTPS_PROXY ||
    process.env.https_proxy;

  if (httpProxy) {
    options.agent = proxyAgent(httpProxy);
  }
  try {
    const response = await fetch(META_ALPHA_VERSION_URL, {
      ...options,
      method: "GET",
    });
    if (!response.ok) {
      throw new Error(
        `failed to fetch version for "clash-meta-alpha" (url="${META_ALPHA_VERSION_URL}", target="${SIDECAR_HOST}", status=${response.status} statusText="${response.statusText}")`
      );
    }
    let v = await response.text();
    META_ALPHA_VERSION = v.trim(); // Trim to remove extra whitespaces
    console.log(`Latest alpha version: ${META_ALPHA_VERSION}`);
  } catch (error) {
    console.error("Error fetching latest alpha version:", error.message);
    process.exit(1);
  }
}

/* ======= clash meta stable ======= */
const META_VERSION_URL = DOWNLOAD_SOURCES.mihomoStableVersion;
const META_URL_PREFIX = DOWNLOAD_SOURCES.mihomoStablePrefix;
// Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
let META_VERSION;

const META_MAP = {
  "win32-x64": "mihomo-windows-amd64-compatible",
};

// Fetch the latest release version from the version.txt file
async function getLatestReleaseVersion() {
  const options = {};

  const httpProxy =
    process.env.HTTP_PROXY ||
    process.env.http_proxy ||
    process.env.HTTPS_PROXY ||
    process.env.https_proxy;

  if (httpProxy) {
    options.agent = proxyAgent(httpProxy);
  }
  try {
    const response = await fetch(META_VERSION_URL, {
      ...options,
      method: "GET",
    });
    if (!response.ok) {
      throw new Error(
        `failed to fetch version for "clash-meta" (url="${META_VERSION_URL}", target="${SIDECAR_HOST}", status=${response.status} statusText="${response.statusText}")`
      );
    }
    let v = await response.text();
    META_VERSION = v.trim(); // Trim to remove extra whitespaces
    console.log(`Latest release version: ${META_VERSION}`);
  } catch (error) {
    console.error("Error fetching latest release version:", error.message);
    process.exit(1);
  }
}

/*
 * check available
 */
if (!META_MAP[`${platform}-${arch}`]) {
  throw new Error(
    `clash meta alpha unsupported platform "${platform}-${arch}"`
  );
}

if (!META_ALPHA_MAP[`${platform}-${arch}`]) {
  throw new Error(
    `clash meta alpha unsupported platform "${platform}-${arch}"`
  );
}

/**
 * core info
 */
function clashMetaAlpha() {
  const name = META_ALPHA_MAP[`${platform}-${arch}`];
  const isWin = platform === "win32";
  const urlExt = isWin ? "zip" : "gz";
  const downloadURL = `${META_ALPHA_URL_PREFIX}/${name}-${META_ALPHA_VERSION}.${urlExt}`;
  const exeFile = `${name}${isWin ? ".exe" : ""}`;
  const zipFile = `${name}-${META_ALPHA_VERSION}.${urlExt}`;

  return {
    name: "clash-meta-alpha",
    targetFile: `clash-meta-alpha-${SIDECAR_HOST}${isWin ? ".exe" : ""}`,
    exeFile,
    zipFile,
    downloadURL,
  };
}

function clashMeta() {
  const name = META_MAP[`${platform}-${arch}`];
  const isWin = platform === "win32";
  const urlExt = isWin ? "zip" : "gz";
  const downloadURL = `${META_URL_PREFIX}/${META_VERSION}/${name}-${META_VERSION}.${urlExt}`;
  const exeFile = `${name}${isWin ? ".exe" : ""}`;
  const zipFile = `${name}-${META_VERSION}.${urlExt}`;

  return {
    name: "clash-meta",
    targetFile: `clash-meta-${SIDECAR_HOST}${isWin ? ".exe" : ""}`,
    exeFile,
    zipFile,
    downloadURL,
  };
}
/**
 * download sidecar and rename
 */
async function resolveSidecar(binInfo) {
  const { name, targetFile, zipFile, exeFile, downloadURL } = binInfo;

  const sidecarDir = path.join(cwd, "src-tauri", "sidecar");
  const sidecarPath = path.join(sidecarDir, targetFile);

  await fs.mkdirp(sidecarDir);
  if (!FORCE && (await fs.pathExists(sidecarPath))) return;

  const tempDir = path.join(TEMP_DIR, name);
  const tempZip = path.join(tempDir, zipFile);
  const tempExe = path.join(tempDir, exeFile);
  console.log(
    `[INFO]: resolving sidecar "${name}" target=${SIDECAR_HOST} url="${downloadURL}" targetPath="${sidecarPath}" tempZip="${tempZip}"`
  );

  await fs.mkdirp(tempDir);
  try {
    if (!(await fs.pathExists(tempZip))) {
      await downloadFile(downloadURL, tempZip);
    }

    if (zipFile.endsWith(".zip")) {
      const zip = new AdmZip(tempZip);
      zip.getEntries().forEach((entry) => {
        console.log(`[DEBUG]: "${name}" entry name`, entry.entryName);
      });
      zip.extractAllTo(tempDir, true);
      if (!(await fs.pathExists(tempExe))) {
        throw new Error(
          `missing extracted executable for "${name}" (url="${downloadURL}", expected="${tempExe}", target="${SIDECAR_HOST}")`
        );
      }
      await fs.rename(tempExe, sidecarPath);
      console.log(`[INFO]: "${name}" unzip finished`);
    } else if (zipFile.endsWith(".tgz")) {
      // tgz
      await fs.mkdirp(tempDir);
      await tar.extract({
        cwd: tempDir,
        file: tempZip,
        //strip: 1, // 可能需要根据实际的 .tgz 文件结构调整
      });
      const files = await fs.readdir(tempDir);
      console.log(`[DEBUG]: "${name}" files in tempDir:`, files);
      const extractedFile = files.find((file) => file.startsWith("虚空终端-"));
      if (extractedFile) {
        const extractedFilePath = path.join(tempDir, extractedFile);
        await fs.rename(extractedFilePath, sidecarPath);
        console.log(`[INFO]: "${name}" file renamed to "${sidecarPath}"`);
        execSync(`chmod 755 ${sidecarPath}`);
        console.log(`[INFO]: "${name}" chmod binary finished`);
      } else {
        throw new Error(
          `expected extracted file not found for "${name}" (url="${downloadURL}", tempDir="${tempDir}", target="${SIDECAR_HOST}")`
        );
      }
    } else {
      // gz
      const readStream = fs.createReadStream(tempZip);
      const writeStream = fs.createWriteStream(sidecarPath);
      await new Promise((resolve, reject) => {
        const onError = (error) => {
          console.error(`[ERROR]: "${name}" gz failed:`, error.message);
          reject(error);
        };
        readStream
          .pipe(zlib.createGunzip().on("error", onError))
          .pipe(writeStream)
          .on("finish", () => {
            console.log(`[INFO]: "${name}" gunzip finished`);
            execSync(`chmod 755 ${sidecarPath}`);
            console.log(`[INFO]: "${name}" chmod binary finished`);
            resolve();
          })
          .on("error", onError);
      });
    }
  } catch (err) {
    // 需要删除文件
    await fs.remove(sidecarPath);
    throw err;
  } finally {
    // delete temp dir
    await fs.remove(tempDir);
  }
}

/**
 * download the file to the resources dir
 */
async function resolveResource(binInfo) {
  const { file, downloadURL } = binInfo;

  const resDir = path.join(cwd, "src-tauri/resources");
  const targetPath = path.join(resDir, file);
  console.log(
    `[INFO]: resolving resource "${file}" url="${downloadURL}" targetPath="${targetPath}" target="${SIDECAR_HOST}"`
  );

  if (!FORCE && (await fs.pathExists(targetPath))) return;

  await fs.mkdirp(resDir);
  await downloadFile(downloadURL, targetPath);

  console.log(`[INFO]: ${file} finished`);
}

/**
 * copy local windows service binaries to resources dir
 */
async function buildWindowsServiceBinariesIfNeeded() {
  if (process.platform !== "win32") return;
  const manifestPath = path.join(cwd, "src-tauri", "windows-service-src", "Cargo.toml");
  const outDir = path.join(cwd, "src-tauri", "local-binaries", "windows-service-bin");
  await fs.mkdirp(outDir);
  execSync(`cargo build --manifest-path "${manifestPath}" --release --target ${SIDECAR_HOST || "x86_64-pc-windows-msvc"}`, { stdio: "inherit" });
  const binDir = path.join(cwd, "src-tauri", "windows-service-src", "target", SIDECAR_HOST || "x86_64-pc-windows-msvc", "release");
  for (const f of ["clash-verge-service.exe", "install-service.exe", "uninstall-service.exe"]) {
    await fs.copy(path.join(binDir, f), path.join(outDir, f), { overwrite: true });
  }
}

async function copyLocalWindowsServiceBinaries() {
  await buildWindowsServiceBinariesIfNeeded();
  const sourceDir = path.join(cwd, "src-tauri", "local-binaries", "windows-service-bin");
  const targetDir = path.join(cwd, "src-tauri", "resources");

  const files = [
    // Windows service binary keeps historical filename for CI/local-binaries compatibility.
    "clash-verge-service.exe",
    "install-service.exe",
    "uninstall-service.exe",
  ];

  await fs.mkdirp(targetDir);

  const removeIfExists = async (filePath) => {
    try {
      await fs.unlink(filePath);
    } catch (err) {
      if (err?.code !== "ENOENT") throw err;
    }
  };

  for (const file of files) {
    const src = path.join(sourceDir, file);
    const dst = path.join(targetDir, file);

    if (!(await fs.pathExists(src))) {
      throw new Error(`Missing local service binary: ${src}`);
    }

    if (!FORCE && (await fs.pathExists(dst))) continue;

    await removeIfExists(dst);
    await fs.copyFile(src, dst);
    console.log(`[INFO]: ${file} copied from local repository`);
  }
}

/**
 * download file and save to `path`
 */
async function downloadFile(url, path) {
  const options = {};

  const httpProxy =
    process.env.HTTP_PROXY ||
    process.env.http_proxy ||
    process.env.HTTPS_PROXY ||
    process.env.https_proxy;

  if (httpProxy) {
    options.agent = proxyAgent(httpProxy);
  }

  console.log(`[INFO]: downloading url="${url}" -> "${path}" target="${SIDECAR_HOST}"`);
  const response = await fetch(url, {
    ...options,
    method: "GET",
    headers: { "Content-Type": "application/octet-stream" },
  });
  if (!response.ok) {
    throw new Error(
      `download failed url="${url}" targetPath="${path}" target="${SIDECAR_HOST}" status=${response.status} statusText="${response.statusText}"`
    );
  }
  const buffer = await response.arrayBuffer();
  await fs.writeFile(path, new Uint8Array(buffer));

  console.log(`[INFO]: download finished "${url}"`);
}

// SimpleSC.dll
const resolvePlugin = async () => {
  const name = "SimpleSC";
  const url = DOWNLOAD_SOURCES.simpleScZip;
  // TODO: add SHA256 checksum for fixed external asset

  const tempDir = path.join(TEMP_DIR, "SimpleSC");
  const tempZip = path.join(
    tempDir,
    "NSIS_Simple_Service_Plugin_Unicode_1.30.zip"
  );
  const tempDll = path.join(tempDir, "SimpleSC.dll");
  const pluginDir = path.join(process.env.APPDATA, "Local/NSIS");
  const pluginPath = path.join(pluginDir, "SimpleSC.dll");
  console.log(
    `[INFO]: resolving plugin "${name}" url="${url}" targetPath="${pluginPath}" tempZip="${tempZip}" target="${SIDECAR_HOST}"`
  );
  await fs.mkdirp(pluginDir);
  await fs.mkdirp(tempDir);
  if (!FORCE && (await fs.pathExists(pluginPath))) return;
  try {
    if (!(await fs.pathExists(tempZip))) {
      await downloadFile(url, tempZip);
    }
    const zip = new AdmZip(tempZip);
    zip.getEntries().forEach((entry) => {
      console.log(`[DEBUG]: "SimpleSC" entry name`, entry.entryName);
    });
    zip.extractAllTo(tempDir, true);
    if (!(await fs.pathExists(tempDll))) {
      throw new Error(
        `plugin dll missing for "${name}" (url="${url}", expected="${tempDll}", targetPath="${pluginPath}", target="${SIDECAR_HOST}")`
      );
    }
    await fs.copyFile(tempDll, pluginPath);
    console.log(`[INFO]: "SimpleSC" unzip finished`);
  } finally {
    await fs.remove(tempDir);
  }
};

/**
 * main
 */

const resolveService = () => copyLocalWindowsServiceBinaries();
const resolveInstall = () => copyLocalWindowsServiceBinaries();
const resolveUninstall = () => copyLocalWindowsServiceBinaries();
const resolveMmdb = () =>
  resolveResource({
    file: "Country.mmdb",
    // Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
    downloadURL: DOWNLOAD_SOURCES.countryMmdb,
  });
const resolveGeosite = () =>
  resolveResource({
    file: "geosite.dat",
    // Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
    downloadURL: DOWNLOAD_SOURCES.geositeDat,
  });
const resolveGeoIP = () =>
  resolveResource({
    file: "geoip.dat",
    // Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
    downloadURL: DOWNLOAD_SOURCES.geoipDat,
  });
const resolveEnableLoopback = () =>
  resolveResource({
    file: "enableLoopback.exe",
    // Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
    downloadURL: DOWNLOAD_SOURCES.enableLoopback,
  });

const tasks = [
  // { name: "clash", func: resolveClash, retry: 5 },
  {
    name: "clash-meta-alpha",
    func: () =>
      getLatestAlphaVersion().then(() => resolveSidecar(clashMetaAlpha())),
    retry: 5,
  },
  {
    name: "clash-meta",
    func: () =>
      getLatestReleaseVersion().then(() => resolveSidecar(clashMeta())),
    retry: 5,
  },
  { name: "plugin", func: resolvePlugin, retry: 5, winOnly: true },
  { name: "service", func: resolveService, retry: 5, winOnly: true },
  { name: "install", func: resolveInstall, retry: 5, winOnly: true },
  { name: "uninstall", func: resolveUninstall, retry: 5, winOnly: true },
  { name: "mmdb", func: resolveMmdb, retry: 5 },
  { name: "geosite", func: resolveGeosite, retry: 5 },
  { name: "geoip", func: resolveGeoIP, retry: 5 },
  {
    name: "enableLoopback",
    func: resolveEnableLoopback,
    retry: 5,
    winOnly: true,
  },
];

async function runTask() {
  const task = tasks.shift();
  if (!task) return;
  if (task.winOnly && process.platform !== "win32") return runTask();

  for (let i = 0; i < task.retry; i++) {
    try {
      await task.func();
      break;
    } catch (err) {
      console.error(`[ERROR]: task::${task.name} try ${i} ==`, err.message);
      if (i === task.retry - 1) throw err;
    }
  }
  return runTask();
}

runTask();
runTask();
