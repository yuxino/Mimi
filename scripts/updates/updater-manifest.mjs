#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

const PLATFORM_KEYS = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"];

export function expectedUpdaterAssets(version) {
  assertVersion(version);
  return {
    dmg: `mimi_${version}_aarch64.dmg`,
    macArchive: "mimi.app.tar.gz",
    macSignature: "mimi.app.tar.gz.sig",
    intelDmg: `mimi_${version}_x64.dmg`,
    intelArchive: "mimi_x64.app.tar.gz",
    intelSignature: "mimi_x64.app.tar.gz.sig",
    windowsExe: `mimi_${version}_x64-setup.exe`,
    windowsExeSignature: `mimi_${version}_x64-setup.exe.sig`,
    windowsMsi: `mimi_${version}_x64_en-US.msi`,
    windowsMsiSignature: `mimi_${version}_x64_en-US.msi.sig`,
    linuxDeb: `mimi_${version}_amd64.deb`,
    linuxAppImage: `mimi_${version}_amd64.AppImage`,
    linuxAppImageSignature: `mimi_${version}_amd64.AppImage.sig`,
  };
}

export function createUpdaterManifest({
  assetDir,
  version,
  repository,
  tag,
  pubDate,
  notes,
}) {
  assertRepository(repository);
  if (tag !== `v${version}`) {
    throw new Error(`updater tag ${tag} does not match version ${version}`);
  }
  if (!Number.isFinite(Date.parse(pubDate))) {
    throw new Error(`invalid updater publication date: ${pubDate}`);
  }

  const assets = expectedUpdaterAssets(version);
  for (const asset of Object.values(assets)) {
    assertFile(assetDir, asset);
  }

  const releaseBase = `https://github.com/${repository}/releases/download/${tag}`;
  const manifest = {
    version,
    notes: notes.trim(),
    pub_date: new Date(pubDate).toISOString(),
    platforms: {
      "darwin-aarch64": {
        signature: readSignature(assetDir, assets.macSignature),
        url: `${releaseBase}/${encodeURIComponent(assets.macArchive)}`,
      },
      "darwin-x86_64": {
        signature: readSignature(assetDir, assets.intelSignature),
        url: `${releaseBase}/${encodeURIComponent(assets.intelArchive)}`,
      },
      "windows-x86_64": {
        signature: readSignature(assetDir, assets.windowsExeSignature),
        url: `${releaseBase}/${encodeURIComponent(assets.windowsExe)}`,
      },
      "linux-x86_64": {
        signature: readSignature(assetDir, assets.linuxAppImageSignature),
        url: `${releaseBase}/${encodeURIComponent(assets.linuxAppImage)}`,
      },
    },
  };

  verifyUpdaterManifest({ manifest, assetDir, version, repository, tag });
  return manifest;
}

export function verifyUpdaterManifest({
  manifest,
  assetDir,
  version,
  repository,
  tag,
}) {
  const assets = expectedUpdaterAssets(version);
  const keys = Object.keys(manifest.platforms ?? {}).sort();
  if (JSON.stringify(keys) !== JSON.stringify([...PLATFORM_KEYS].sort())) {
    throw new Error(`unexpected updater platforms: ${keys.join(", ")}`);
  }
  if (manifest.version !== version || manifest.pub_date === undefined) {
    throw new Error("updater manifest metadata does not match the release");
  }

  const expected = {
    "darwin-aarch64": assets.macArchive,
    "darwin-x86_64": assets.intelArchive,
    "windows-x86_64": assets.windowsExe,
    "linux-x86_64": assets.linuxAppImage,
  };
  const urls = new Set();
  for (const platform of PLATFORM_KEYS) {
    const entry = manifest.platforms[platform];
    const expectedUrl = `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(expected[platform])}`;
    if (entry.url !== expectedUrl) {
      throw new Error(`${platform} updater URL does not match its release asset`);
    }
    if (urls.has(entry.url)) throw new Error("updater URLs must be unique");
    urls.add(entry.url);

    const signatureName = `${expected[platform]}.sig`;
    if (entry.signature !== readSignature(assetDir, signatureName)) {
      throw new Error(`${platform} updater signature does not match its asset`);
    }
  }
}

function assertVersion(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`invalid updater version: ${version}`);
  }
}

function assertRepository(repository) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository: ${repository}`);
  }
}

function assertFile(assetDir, name) {
  const contents = readFileSync(join(assetDir, name));
  if (contents.length === 0) throw new Error(`empty release asset: ${name}`);
}

function readSignature(assetDir, name) {
  const signature = readFileSync(join(assetDir, name), "utf8").trim();
  if (!signature) throw new Error(`empty updater signature: ${name}`);
  let decoded;
  try {
    decoded = Buffer.from(signature, "base64").toString("utf8");
  } catch {
    throw new Error(`invalid base64 updater signature: ${name}`);
  }
  if (!decoded.includes("untrusted comment:") || !decoded.includes("trusted comment:")) {
    throw new Error(`unexpected minisign signature payload: ${name}`);
  }
  return signature;
}

function usage() {
  throw new Error(
    "usage: updater-manifest.mjs create <asset-dir> <output> <version> <owner/repo> <tag> <pub-date> <notes-file> | verify <asset-dir> <manifest> <version> <owner/repo> <tag>",
  );
}

function main(argv) {
  const [mode, assetDir, manifestPath, version, repository, tag, pubDate, notesPath] = argv;
  if (mode === "create") {
    if (!notesPath) usage();
    const manifest = createUpdaterManifest({
      assetDir,
      version,
      repository,
      tag,
      pubDate,
      notes: readFileSync(notesPath, "utf8"),
    });
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    return;
  }
  if (mode === "verify") {
    if (!tag || pubDate !== undefined) usage();
    verifyUpdaterManifest({
      manifest: JSON.parse(readFileSync(manifestPath, "utf8")),
      assetDir,
      version,
      repository,
      tag,
    });
    return;
  }
  usage();
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`${basename(process.argv[1])}: ${error.message}`);
    process.exitCode = 1;
  }
}
