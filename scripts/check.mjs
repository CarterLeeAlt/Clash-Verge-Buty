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

function getHttpProxyAgent() {
  const httpProxy =
    process.env.HTTP_PROXY ||
    process.env.http_proxy ||
    process.env.HTTPS_PROXY ||
    process.env.https_proxy;

  return httpProxy ? proxyAgent(httpProxy) : undefined;
}

function readPositiveNumber(value, fallback) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

async function fetchWithRetry(url, options = {}, context = {}) {
  const {
    retries: configuredRetries,
    retryDelayMs: configuredRetryDelayMs,
    timeoutMs: configuredTimeoutMs,
    targetPath,
    name,
  } = context;
  const retries = Math.max(
    5,
    Math.floor(
      readPositiveNumber(
        configuredRetries ?? process.env.VERGE_DOWNLOAD_RETRY,
        5
      )
    )
  );
  const retryDelayMs = readPositiveNumber(
    configuredRetryDelayMs ?? process.env.VERGE_DOWNLOAD_RETRY_DELAY_MS,
    3000
  );
  const timeoutMs = readPositiveNumber(
    configuredTimeoutMs ?? process.env.VERGE_DOWNLOAD_TIMEOUT_MS,
    60000
  );
  const targetPathLog = targetPath ? ` targetPath="${targetPath}"` : "";
  const nameLog = name ? ` name="${name}"` : "";
  let lastError;

  for (let attempt = 1; attempt <= retries; attempt++) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);

    try {
      console.log(
        `[INFO]: fetching attempt ${attempt}/${retries} url="${url}" target="${SIDECAR_HOST}"${targetPathLog}${nameLog}`
      );

      const response = await fetch(url, {
        ...options,
        signal: controller.signal,
      });

      if (!response.ok) {
        throw new Error(
          `request failed url="${url}" target="${SIDECAR_HOST}" status=${response.status} statusText="${response.statusText}"${targetPathLog}${nameLog}`
        );
      }

      return response;
    } catch (err) {
      lastError = err;
      console.warn(
        `[WARN]: fetch failed attempt ${attempt}/${retries} url="${url}" target="${SIDECAR_HOST}"${targetPathLog}${nameLog} error="${err?.message || err}"`
      );

      if (attempt >= retries) break;

      await new Promise((resolve) =>
        setTimeout(resolve, retryDelayMs * attempt)
      );
    } finally {
      clearTimeout(timer);
    }
  }

  throw lastError;
}

const DOWNLOAD_SOURCES = {
  mihomoAlphaVersion:
    "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha/version.txt",
  mihomoAlphaPrefix:
    "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha",
  mihomoStableVersion:
    "https://github.com/MetaCubeX/mihomo/releases/latest/download/version.txt",
  mihomoStablePrefix: "https://github.com/MetaCubeX/mihomo/releases/download",
  countryMmdb:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
  geositeDat:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geosite.dat",
  geoipDat:
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat",
  enableLoopback:
    "https://github.com/Kuingsmile/uwp-tool/releases/download/latest/enableLoopback.exe",
};

/* ======= mihomo alpha ======= */
const META_ALPHA_VERSION_URL = DOWNLOAD_SOURCES.mihomoAlphaVersion;
const META_ALPHA_URL_PREFIX = DOWNLOAD_SOURCES.mihomoAlphaPrefix;
// Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
let META_ALPHA_VERSION;

const META_ALPHA_MAP = {
  "win32-x64": "mihomo-windows-amd64-v2",
};

// Fetch the latest alpha release version from the version.txt file
async function getLatestAlphaVersion() {
  const agent = getHttpProxyAgent();
  const options = agent ? { agent } : {};
  const response = await fetchWithRetry(
    META_ALPHA_VERSION_URL,
    {
      ...options,
      method: "GET",
    },
    {
      name: "mihomo-alpha-version",
    }
  );
  let v = await response.text();
  META_ALPHA_VERSION = v.trim(); // Trim to remove extra whitespaces
  console.log(`Latest alpha version: ${META_ALPHA_VERSION}`);
}

/* ======= mihomo stable ======= */
const META_VERSION_URL = DOWNLOAD_SOURCES.mihomoStableVersion;
const META_URL_PREFIX = DOWNLOAD_SOURCES.mihomoStablePrefix;
// Dynamic upstream asset; checksum cannot be fixed unless version is pinned.
let META_VERSION;

const META_MAP = {
  "win32-x64": "mihomo-windows-amd64-v2",
};

// Fetch the latest release version from the version.txt file
async function getLatestReleaseVersion() {
  const agent = getHttpProxyAgent();
  const options = agent ? { agent } : {};
  const response = await fetchWithRetry(
    META_VERSION_URL,
    {
      ...options,
      method: "GET",
    },
    {
      name: "mihomo-version",
    }
  );
  let v = await response.text();
  META_VERSION = v.trim(); // Trim to remove extra whitespaces
  console.log(`Latest release version: ${META_VERSION}`);
}

/*
 * check available
 */
if (!META_MAP[`${platform}-${arch}`]) {
  throw new Error(
    `mihomo unsupported platform "${platform}-${arch}"`
  );
}

if (!META_ALPHA_MAP[`${platform}-${arch}`]) {
  throw new Error(
    `mihomo alpha unsupported platform "${platform}-${arch}"`
  );
}

/**
 * core info
 */
function mihomoAlpha() {
  const name = META_ALPHA_MAP[`${platform}-${arch}`];
  const isWin = platform === "win32";
  const urlExt = isWin ? "zip" : "gz";
  const downloadURL = `${META_ALPHA_URL_PREFIX}/${name}-${META_ALPHA_VERSION}.${urlExt}`;
  const exeFile = `${name}${isWin ? ".exe" : ""}`;
  const zipFile = `${name}-${META_ALPHA_VERSION}.${urlExt}`;

  return {
    name: "mihomo-alpha",
    targetFile: `mihomo-alpha-${SIDECAR_HOST}${isWin ? ".exe" : ""}`,
    exeFile,
    zipFile,
    downloadURL,
  };
}

function mihomoStable() {
  const name = META_MAP[`${platform}-${arch}`];
  const isWin = platform === "win32";
  const urlExt = isWin ? "zip" : "gz";
  const downloadURL = `${META_URL_PREFIX}/${META_VERSION}/${name}-${META_VERSION}.${urlExt}`;
  const exeFile = `${name}${isWin ? ".exe" : ""}`;
  const zipFile = `${name}-${META_VERSION}.${urlExt}`;

  return {
    name: "mihomo",
    targetFile: `mihomo-${SIDECAR_HOST}${isWin ? ".exe" : ""}`,
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
const WINDOWS_SERVICE_BINARY_FILES = [
  // Windows service binary keeps historical filename for CI/local-binaries compatibility.
  "clash-verge-service.exe",
  "install-service.exe",
  "uninstall-service.exe",
];

let windowsServiceBuildPromise = null;
let windowsServiceBinariesPromise = null;

function buildWindowsServiceBinariesIfNeededOnce() {
  if (!windowsServiceBuildPromise) {
    windowsServiceBuildPromise = buildWindowsServiceBinariesIfNeeded().catch(
      (err) => {
        windowsServiceBuildPromise = null;
        throw err;
      }
    );
  }
  return windowsServiceBuildPromise;
}

async function buildWindowsServiceBinariesIfNeeded() {
  if (process.platform !== "win32") return;

  const serviceTarget = SIDECAR_HOST || "x86_64-pc-windows-msvc";
  const manifestPath = path.join(
    cwd,
    "src-tauri",
    "windows-service-src",
    "Cargo.toml"
  );
  const outDir = path.join(
    cwd,
    "src-tauri",
    "local-binaries",
    "windows-service-bin"
  );
  const binDir = path.join(
    cwd,
    "src-tauri",
    "windows-service-src",
    "target",
    serviceTarget,
    "release"
  );

  console.log(
    `[INFO]: building Windows service binaries once target=${serviceTarget} files=${WINDOWS_SERVICE_BINARY_FILES.join(
      ","
    )}`
  );
  await fs.mkdirp(outDir);
  execSync(
    `cargo build --manifest-path "${manifestPath}" --release --target ${serviceTarget}`,
    { stdio: "inherit" }
  );

  for (const file of WINDOWS_SERVICE_BINARY_FILES) {
    const src = path.join(binDir, file);
    const dst = path.join(outDir, file);

    if (!(await fs.pathExists(src))) {
      throw new Error(`Missing built service binary: ${src}`);
    }

    await fs.copy(src, dst, { overwrite: true });
  }
}

function copyLocalWindowsServiceBinariesOnce() {
  if (!windowsServiceBinariesPromise) {
    windowsServiceBinariesPromise = copyLocalWindowsServiceBinaries().catch(
      (err) => {
        windowsServiceBinariesPromise = null;
        throw err;
      }
    );
  }
  return windowsServiceBinariesPromise;
}

async function copyLocalWindowsServiceBinaries() {
  console.log(
    `[INFO]: resolving Windows service binaries as one task files=${WINDOWS_SERVICE_BINARY_FILES.join(
      ","
    )}`
  );
  await buildWindowsServiceBinariesIfNeededOnce();

  const sourceDir = path.join(
    cwd,
    "src-tauri",
    "local-binaries",
    "windows-service-bin"
  );
  const targetDir = path.join(cwd, "src-tauri", "resources");

  await fs.mkdirp(targetDir);

  const removeIfExists = async (filePath) => {
    try {
      await fs.unlink(filePath);
    } catch (err) {
      if (err?.code !== "ENOENT") throw err;
    }
  };

  for (const file of WINDOWS_SERVICE_BINARY_FILES) {
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

  console.log(
    `[INFO]: Windows service binaries ready in resources: ${WINDOWS_SERVICE_BINARY_FILES.join(
      ","
    )}`
  );
}

/**
 * download file and save to `path`
 */
async function downloadFile(url, path) {
  const agent = getHttpProxyAgent();
  const options = agent ? { agent } : {};
  const tempPath = `${path}.download`;

  console.log(
    `[INFO]: downloading url="${url}" -> "${path}" target="${SIDECAR_HOST}"`
  );

  try {
    const response = await fetchWithRetry(
      url,
      {
        ...options,
        method: "GET",
        headers: { "Content-Type": "application/octet-stream" },
      },
      {
        name: "download-file",
        targetPath: path,
      }
    );
    const buffer = await response.arrayBuffer();
    await fs.writeFile(tempPath, new Uint8Array(buffer));
    await fs.rename(tempPath, path);

    console.log(`[INFO]: download finished "${url}"`);
  } catch (err) {
    await fs.remove(tempPath).catch(() => {});
    throw err;
  }
}

/**
 * main
 */

const resolveWindowsServiceBinaries = () =>
  copyLocalWindowsServiceBinariesOnce();
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
    name: "mihomo-alpha",
    func: () =>
      getLatestAlphaVersion().then(() => resolveSidecar(mihomoAlpha())),
    retry: 5,
  },
  {
    name: "mihomo",
    func: () =>
      getLatestReleaseVersion().then(() => resolveSidecar(mihomoStable())),
    retry: 5,
  },
  {
    name: "windows-service-binaries",
    func: resolveWindowsServiceBinaries,
    retry: 5,
    winOnly: true,
  },
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
