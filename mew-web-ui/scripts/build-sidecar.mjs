import { chmod, copyFile, mkdir } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const webUiRoot = resolve(scriptDir, "..");
const repoRoot = resolve(webUiRoot, "..");
const targetFromTauri = process.env.TAURI_ENV_TARGET_TRIPLE;
const target =
  targetFromTauri ??
  execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const binaryName = process.platform === "win32" ? "mew.exe" : "mew";
const cargoArgs = ["build", "-p", "mew"];

if (release) {
  cargoArgs.push("--release");
}
if (targetFromTauri) {
  cargoArgs.push("--target", target);
}

execFileSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
});

const outputRoot = targetFromTauri
  ? join(repoRoot, "target", target, profile)
  : join(repoRoot, "target", profile);
const source = join(outputRoot, binaryName);
const binariesDir = join(webUiRoot, "src-tauri", "binaries");
const destination = join(
  binariesDir,
  `mew-${target}${process.platform === "win32" ? ".exe" : ""}`,
);

await mkdir(binariesDir, { recursive: true });
await copyFile(source, destination);
if (process.platform !== "win32") {
  await chmod(destination, 0o755);
}

console.log(`prepared ${destination}`);
