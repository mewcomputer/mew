import { chmod, copyFile, mkdir } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const webUiRoot = resolve(scriptDir, "..");
const repoRoot = resolve(webUiRoot, "..");
const cefRoot = join(repoRoot, "native", "cef-host");
const targetFromTauri = process.env.TAURI_ENV_TARGET_TRIPLE;
const target =
  targetFromTauri ??
  execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const binaryName = process.platform === "win32"
  ? "mew-cef-host-helper.exe"
  : "mew-cef-host-helper";
const cargoArgs = [
  "build",
  "--manifest-path",
  join(cefRoot, "Cargo.toml"),
  "--bin",
  "mew-cef-host-helper",
  "--no-default-features",
];

if (release) cargoArgs.push("--release");
if (targetFromTauri) cargoArgs.push("--target", target);

execFileSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
});

const outputRoot = targetFromTauri
  ? join(cefRoot, "target", target, profile)
  : join(cefRoot, "target", profile);
const source = join(outputRoot, binaryName);
const binariesDir = join(webUiRoot, "src-tauri", "binaries");
const destination = join(
  binariesDir,
  `mew-cef-host-helper-${target}${process.platform === "win32" ? ".exe" : ""}`,
);

await mkdir(binariesDir, { recursive: true });
await copyFile(source, destination);
if (process.platform !== "win32") {
  await chmod(destination, 0o755);
}

console.log(`prepared ${destination}`);
